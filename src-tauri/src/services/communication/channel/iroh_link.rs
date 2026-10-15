//! `IrohDataLink`: a `DataLink` over one QUIC stream of an iroh connection
//! (ADR-0009 D9). The connection's TLS handshake proved the peer's key, which
//! the link reports as `peer_key` so the gate can check it against the pair.
//!
//! Frames are length-prefixed (u32, big-endian) on one bidirectional stream.
//! The dialer opens the stream and writes an empty frame first
//! (`STREAM_START`), which is what makes the stream visible to the
//! acceptor; `recv` skips it. Liveness is QUIC's own keep-alive and idle
//! timeout, iroh's defaults; there is no ping frame.

use async_trait::async_trait;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::time::Duration;

use super::{ChannelKind, DataLink};

/// The ALPN of the frames that come before a session: pairing, adding a
/// channel, identifying a device.
pub const PEER_ALPN: &[u8] = b"fini/peer/1";

/// The ALPN of a sync session. Its last part is the protocol version: a
/// device on another version shares no ALPN with this one, and the TLS
/// handshake fails. Who the peer is, is the key the handshake proved; there
/// is no auth frame (2026-10-13: TLS only).
pub const SYNC_ALPN: &[u8] = b"fini/sync/4";

/// An empty frame: the dialer's first write, so that the acceptor sees the
/// stream before the first real frame. Not a message; `recv` skips it.
const STREAM_START: [u8; 4] = [0, 0, 0, 0];

/// Largest frame accepted from a peer. A full sync batch is far below this;
/// the cap stops a broken or hostile peer from making this device allocate
/// without bound.
const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// How long a closing link waits for the peer to take what was sent last.
/// Dropping a QUIC connection discards data still in flight, which would
/// lose the last frame of a reply (an `AddChannelAccept`).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

pub struct IrohDataLink {
    kind: ChannelKind,
    connection: Option<Connection>,
    send: Option<SendStream>,
    recv: RecvStream,
    buffer: Vec<u8>,
    peer_addr: Option<String>,
    peer_key: String,
}

impl IrohDataLink {
    pub fn new(
        kind: ChannelKind,
        connection: Connection,
        send: SendStream,
        recv: RecvStream,
        peer_addr: Option<String>,
    ) -> Self {
        let peer_key = connection.remote_id().to_string();
        Self {
            kind,
            connection: Some(connection),
            send: Some(send),
            recv,
            buffer: Vec::new(),
            peer_addr,
            peer_key,
        }
    }

    /// Opens the stream on a connection this device dialled.
    pub async fn open(
        kind: ChannelKind,
        connection: Connection,
        peer_addr: Option<String>,
    ) -> Result<Self, String> {
        let (mut send, recv) = connection
            .open_bi()
            .await
            .map_err(|err| format!("opening a stream failed: {err}"))?;
        send.write_all(&STREAM_START)
            .await
            .map_err(|err| format!("opening a stream failed: {err}"))?;
        Ok(Self::new(kind, connection, send, recv, peer_addr))
    }

    /// Accepts the stream on a connection the peer dialled.
    pub async fn accept(
        kind: ChannelKind,
        connection: Connection,
        peer_addr: Option<String>,
    ) -> Result<Self, String> {
        let (send, recv) = connection
            .accept_bi()
            .await
            .map_err(|err| format!("accepting a stream failed: {err}"))?;
        Ok(Self::new(kind, connection, send, recv, peer_addr))
    }

    /// Ends this side's stream and waits, briefly, until the peer has
    /// everything sent on it. For one-shot senders that must not return
    /// before their frame is delivered.
    pub async fn finish(mut self) {
        if let Some(mut send) = self.send.take() {
            let _ = send.finish();
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, send.stopped()).await;
        }
    }

    /// The next whole frame already in the buffer, if any. Skips the
    /// dialer's `STREAM_START`.
    fn take_frame(&mut self) -> Option<Result<Vec<u8>, String>> {
        while self.buffer.starts_with(&STREAM_START) {
            self.buffer.drain(..STREAM_START.len());
        }
        let header: [u8; 4] = self.buffer.get(..4)?.try_into().ok()?;
        let len = u32::from_be_bytes(header) as usize;
        if len > MAX_FRAME_LEN {
            return Some(Err(format!(
                "frame of {len} bytes is over the {MAX_FRAME_LEN}-byte limit"
            )));
        }
        if self.buffer.len() < 4 + len {
            return None;
        }
        let frame = self.buffer[4..4 + len].to_vec();
        self.buffer.drain(..4 + len);
        Some(Ok(frame))
    }
}

#[async_trait]
impl DataLink for IrohDataLink {
    fn kind(&self) -> ChannelKind {
        self.kind
    }

    fn peer_addr(&self) -> Option<String> {
        self.peer_addr.clone()
    }

    fn peer_key(&self) -> Option<String> {
        Some(self.peer_key.clone())
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn close(&mut self, code: u32, reason: &str) {
        if let Some(connection) = self.connection.take() {
            connection.close(iroh::endpoint::VarInt::from_u32(code), reason.as_bytes());
        }
        self.send = None;
    }

    fn close_reason(&self) -> Option<String> {
        self.connection.as_ref()?.close_reason().map(|reason| reason.to_string())
    }

    fn close_code(&self) -> Option<u32> {
        match self.connection.as_ref()?.close_reason()? {
            iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
                u32::try_from(close.error_code.into_inner()).ok()
            }
            _ => None,
        }
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        let send = self.send.as_mut().ok_or("link closed")?;
        let len = u32::try_from(payload.len()).map_err(|_| "frame too large".to_string())?;
        let mut framed = Vec::with_capacity(4 + payload.len());
        framed.extend_from_slice(&len.to_be_bytes());
        framed.extend_from_slice(&payload);
        send.write_all(&framed).await.map_err(|err| err.to_string())
    }

    /// Cancel-safe: `RecvStream::read` is, and partial frames stay in
    /// `buffer`, so a `select!` that drops this future loses nothing.
    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            if let Some(frame) = self.take_frame() {
                return Some(frame);
            }
            match self.recv.read(&mut chunk).await {
                Ok(Some(read)) => self.buffer.extend_from_slice(&chunk[..read]),
                // Finished, reset, or the connection went: the link is over.
                Ok(None) | Err(_) => return None,
            }
        }
    }
}

impl Drop for IrohDataLink {
    fn drop(&mut self) {
        let (Some(connection), Some(mut send)) = (self.connection.take(), self.send.take()) else {
            return;
        };
        let _ = send.finish();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Keep the connection until the peer has the last frame, or a short
        // while has passed.
        runtime.spawn(async move {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, send.stopped()).await;
            drop(connection);
        });
    }
}
