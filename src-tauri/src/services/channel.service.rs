//! The channel interface (ADR-0010 D10): what every channel service can do,
//! and the parts all channels share -- the `DataLink` a channel gives, the
//! frame codec, and the interfaces a channel hands its links to.
//!
//! A channel service carries bytes to paired devices and knows nothing
//! about devices or pairing (D15). It hands each link to the handler that
//! the app start registered for its ALPN (`LinkHandlers`): pairing links to
//! `DeviceService`, sessions to `SpaceSyncService`. The channel layer
//! defines these interfaces, so a channel never depends on a feature
//! service.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

pub use crate::models::channel::ChannelKind;
use crate::models::channel::ChannelProblem;
use crate::models::pairing::DiscoveredDevice;
use crate::models::peer_frame::PeerFrame;
use crate::repositories::ChannelRepository;

/// A live, point-to-point connection to one peer. Moves whole byte
/// messages; framing is the link's concern, not the caller's.
#[async_trait]
pub trait DataLink: Send {
    fn kind(&self) -> ChannelKind;
    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String>;
    /// `None` means the link closed.
    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>>;
    /// The peer's address, if the link has one: an IP address, or a
    /// Bluetooth address.
    fn peer_addr(&self) -> Option<String> {
        None
    }

    /// The peer's iroh key (`EndpointId`, hex), when the link's TLS
    /// handshake proved one (ADR-0009 D8).
    fn peer_key(&self) -> Option<String> {
        None
    }

    /// Ends the link with an application close `code` the peer can read:
    /// how a session is rejected. Links without close codes just end.
    fn close(&mut self, _code: u32, _reason: &str) {}

    /// Why the peer closed the link, when the link knows: for the logs.
    fn close_reason(&self) -> Option<String> {
        None
    }

    /// The application close code the peer closed the link with, if any.
    fn close_code(&self) -> Option<u32> {
        None
    }
}

/// Sends one `PeerFrame` over a link.
pub async fn send_frame(link: &mut dyn DataLink, frame: &PeerFrame) -> Result<(), String> {
    let bytes = codec::encode_frame(frame)?;
    link.send(bytes).await
}

/// Receives one `PeerFrame` from a link. `None` means the link closed;
/// `Some(Err(_))` means a frame that could not be read.
pub async fn recv_frame(link: &mut dyn DataLink) -> Option<Result<PeerFrame, String>> {
    match link.recv().await? {
        Ok(bytes) => Some(codec::decode_frame(&bytes)),
        Err(err) => Some(Err(err)),
    }
}

/// Takes the links of the frames before a session (`PEER_ALPN`, and every
/// plain Bluetooth link): pairing, adding a channel, identifying a device.
/// `DeviceService` implements it.
#[async_trait]
pub trait PeerLinkHandler: Send + Sync {
    async fn accept(&self, link: Box<dyn DataLink>);
}

/// Takes the links of sync sessions (`SYNC_ALPN`). `SpaceSyncService`
/// implements it.
#[async_trait]
pub trait SessionHandler: Send + Sync {
    /// Whether a session with the peer runs on this channel now.
    fn has_session(&self, peer_device_id: &str, kind: ChannelKind) -> bool;

    /// Ends the session with the peer on this channel, if one runs.
    fn close_session(&self, peer_device_id: &str, kind: ChannelKind) -> bool;

    /// A session link that this device connected: runs the session on it
    /// until it ends. `on_held` runs once the session is claimed.
    /// `Err(link)` gives the link back when a session runs already. Returns
    /// the close code the peer ended the link with, if it did.
    async fn hold<'a>(
        &'a self,
        link: Box<dyn DataLink>,
        peer_device_id: &'a str,
        kind: ChannelKind,
        on_held: Box<dyn FnOnce() + Send + 'a>,
    ) -> Result<Option<u32>, Box<dyn DataLink>>;

    /// A session link that the peer connected: checks who it is and runs
    /// the session, or rejects it with a close code.
    async fn accept(&self, link: Box<dyn DataLink>);
}

/// The handlers the app start registers on each channel, last (D15).
#[derive(Clone)]
pub struct LinkHandlers {
    pub peer: Arc<dyn PeerLinkHandler>,
    pub session: Arc<dyn SessionHandler>,
}

/// Holds the `LinkHandlers` of a channel. Set once, at app start; a link
/// that arrives before that is dropped.
#[derive(Default)]
pub struct LinkHandlerSlot(std::sync::OnceLock<LinkHandlers>);

impl LinkHandlerSlot {
    pub fn set(&self, handlers: LinkHandlers) {
        let _ = self.0.set(handlers);
    }

    pub fn get(&self) -> Option<&LinkHandlers> {
        self.0.get()
    }

    /// Whether a session with the peer runs on this channel now. `false`
    /// before the handlers are set.
    pub fn has_session(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        self.get().is_some_and(|handlers| handlers.session.has_session(peer_device_id, kind))
    }

    /// Ends the session with the peer on this channel, if one runs.
    pub fn close_session(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        self.get().is_some_and(|handlers| handlers.session.close_session(peer_device_id, kind))
    }
}

/// Everything one channel service can do. Implemented by
/// `NetworkChannelService` and `BluetoothChannelService`.
#[async_trait]
pub trait ChannelService: Send + Sync {
    /// The kind of this channel.
    fn kind(&self) -> ChannelKind;

    /// The database this channel reads the pairs from.
    fn db_path(&self) -> &Path;

    /// Whether the pair with `peer_device_id` has this channel and it is on
    /// now, read from the database each time. A removed pair, a removed
    /// channel, and a channel that is off all give `false`.
    fn is_on(&self, peer_device_id: &str) -> bool {
        tokio::task::block_in_place(|| {
            let mut conn = crate::services::db::open_db_at_path(self.db_path());
            ChannelRepository::new(&mut conn).is_enabled(peer_device_id, self.kind())
        })
    }

    /// The handlers of the links this channel gives (D15). The app start
    /// sets them, last.
    fn set_link_handlers(&self, handlers: LinkHandlers);

    /// Whether this channel can work on this device at all: a platform
    /// fact.
    fn available(&self) -> bool;

    /// Whether this channel can be switched on now (ADR-0008 D6).
    fn usable(&self) -> bool {
        self.available()
    }

    /// Asks the hardware directly, now. Called when a person flips a
    /// switch, never on a regular path.
    async fn probe(&self) -> bool {
        self.available()
    }

    /// Starts this channel's own way of finding other devices.
    fn start_discovery(self: Arc<Self>) {}

    /// Starts the accept loop, so that a peer that connects is answered.
    fn start_serving(self: Arc<Self>) {}

    /// Keeps a session with every paired device that has this channel on:
    /// called on each SpaceSync tick.
    fn keep_serving(self: Arc<Self>) {}

    /// A link to a paired device for the frames before a session
    /// (`PEER_ALPN`), such as `AddChannel`: finds the device, for up to
    /// `within`, and connects to the key pinned for the pair.
    async fn connect_plain(&self, peer_device_id: &str, within: Duration) -> Result<Box<dyn DataLink>, String>;

    /// Whether this device connects to the peer on this channel now: to
    /// settle two connections that cross.
    fn connecting(&self, _peer_device_id: &str) -> bool {
        false
    }

    /// Forgets earlier failed attempts with this peer, so the next one goes
    /// at once.
    fn forget_failures(&self, _peer_device_id: &str) {}

    /// A problem on this device's side of the channel, if any (ADR-0008 D6,
    /// D19).
    fn problem(&self) -> Option<ChannelProblem>;

    /// Add Device opened or closed on this device.
    fn set_add_device_mode(&self, _on: bool) {}

    /// The devices in Add Device that this channel heard.
    fn available_devices(&self) -> Vec<DiscoveredDevice> {
        Vec::new()
    }

    /// Whether a setup of this channel is open on this device: then it must
    /// be reachable (ADR-0008 D8).
    fn set_setup_open(&self, _open: bool) {}
}

/// iroh's `ProtocolHandler` for one ALPN of a channel's endpoint: iroh's
/// `Router` accepts every connection and hands it here once TLS proved the
/// peer's key; this makes the `DataLink` and gives it to the handler.
#[derive(Clone)]
pub struct LinkAcceptor {
    kind: ChannelKind,
    handlers: LinkHandlers,
    /// Whether the channel's test switch is off: then nobody is answered.
    switched_off: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl std::fmt::Debug for LinkAcceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkAcceptor").field("kind", &self.kind).finish()
    }
}

impl LinkAcceptor {
    /// Serves `endpoint` for channel `kind` until the endpoint closes: an
    /// iroh `Router` with one acceptor for each of Fini's ALPNs. The
    /// `Router` stops when dropped, so this holds it.
    pub async fn serve(
        kind: ChannelKind,
        endpoint: iroh::Endpoint,
        handlers: LinkHandlers,
        switched_off: Arc<dyn Fn() -> bool + Send + Sync>,
    ) {
        use iroh_link::{PEER_ALPN, SYNC_ALPN};
        let acceptor = LinkAcceptor { kind, handlers, switched_off };
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(SYNC_ALPN, acceptor.clone())
            .accept(PEER_ALPN, acceptor)
            .spawn();
        endpoint.closed().await;
        let _ = router.shutdown().await;
    }

    /// Where the connection's selected path reaches the peer: an IP address
    /// for Network, a Bluetooth address for Bluetooth.
    fn peer_addr(connection: &iroh::endpoint::Connection) -> Option<String> {
        let paths = connection.paths();
        let path = paths.iter().find(|path| path.is_selected())?;
        match path.remote_addr() {
            iroh::TransportAddr::Ip(addr) => Some(addr.ip().to_string()),
            #[cfg(any(target_os = "linux", target_os = "android"))]
            iroh::TransportAddr::Custom(addr) => ble_gatt_iroh::peer_address(addr).map(|peer| peer.0),
            _ => None,
        }
    }
}

impl iroh::protocol::ProtocolHandler for LinkAcceptor {
    async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), iroh::protocol::AcceptError> {
        use iroh_link::{IrohDataLink, SYNC_ALPN};
        if (self.switched_off)() {
            connection.close(0u32.into(), b"the channel is switched off");
            return Ok(());
        }
        let is_session = connection.alpn() == SYNC_ALPN;
        let peer_addr = Self::peer_addr(&connection);
        let link = match IrohDataLink::accept(self.kind, connection, peer_addr).await {
            Ok(link) => Box::new(link),
            Err(err) => {
                log::warn!("[channel] {:?}: {err}", self.kind);
                return Ok(());
            }
        };
        if is_session {
            self.handlers.session.accept(link).await;
        } else {
            self.handlers.peer.accept(link).await;
        }
        Ok(())
    }
}

/// `PeerFrame <-> bytes`, through the versioned envelope and the active
/// `SecureChannel` (plain today). The same wire form on every channel.
pub mod codec {
    use super::encryption::{PlaintextChannel, SecureChannel};
    use super::envelope::{EncScheme, FrameEnvelope, ENVELOPE_VERSION};
    use crate::models::peer_frame::PeerFrame;

    fn channel() -> impl SecureChannel {
        PlaintextChannel
    }

    /// `PeerFrame` -> payload in an envelope -> bytes, for `DataLink::send`.
    pub fn encode_frame(frame: &PeerFrame) -> Result<Vec<u8>, String> {
        let plain = serde_json::to_vec(frame).map_err(|err| format!("encode PeerFrame: {err}"))?;
        let channel = channel();
        let payload = channel.encrypt(plain)?;
        let envelope = FrameEnvelope::new(channel.scheme(), payload);
        serde_json::to_vec(&envelope).map_err(|err| format!("encode envelope: {err}"))
    }

    /// Bytes from `DataLink::recv` -> envelope -> payload -> `PeerFrame`.
    pub fn decode_frame(bytes: &[u8]) -> Result<PeerFrame, String> {
        let envelope: FrameEnvelope = serde_json::from_slice(bytes).map_err(|err| format!("decode envelope: {err}"))?;
        if envelope.v != ENVELOPE_VERSION {
            return Err(format!("unsupported envelope version {}", envelope.v));
        }
        if envelope.enc != EncScheme::None {
            return Err(format!("unsupported encryption scheme {:?} (no SecureChannel impl yet)", envelope.enc));
        }
        let plain = channel().decrypt(envelope.payload)?;
        serde_json::from_slice(&plain).map_err(|err| format!("decode PeerFrame: {err}"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trips_a_peer_frame() {
            let frame = PeerFrame::Ack { event_id: "e1".to_string() };
            let decoded = decode_frame(&encode_frame(&frame).expect("encode")).expect("decode");
            assert!(matches!(decoded, PeerFrame::Ack { event_id } if event_id == "e1"));
        }

        /// A frame `type` this build does not know decodes to `Unknown`, so
        /// that one frame from a newer peer does not end the session.
        #[test]
        fn an_unknown_frame_type_decodes_to_unknown() {
            let inner = serde_json::json!({ "type": "a_frame_kind_from_a_newer_build", "extra": 1 });
            let envelope = FrameEnvelope::new(EncScheme::None, serde_json::to_vec(&inner).unwrap());
            let decoded = decode_frame(&serde_json::to_vec(&envelope).unwrap()).expect("must decode");
            assert!(matches!(decoded, PeerFrame::Unknown));
        }

        #[test]
        fn rejects_an_unsupported_envelope_version() {
            let envelope = FrameEnvelope {
                v: 99,
                enc: EncScheme::None,
                payload: serde_json::to_vec(&PeerFrame::Ack { event_id: "e1".to_string() }).unwrap(),
            };
            let err = decode_frame(&serde_json::to_vec(&envelope).unwrap()).expect_err("should reject");
            assert!(err.contains("unsupported envelope version"));
        }
    }
}

/// The versioned envelope around every encoded `PeerFrame`, so that real
/// encryption later is an additive wire change.
pub mod envelope {
    use serde::{Deserialize, Serialize};

    pub const ENVELOPE_VERSION: u8 = 1;

    /// Which `SecureChannel` produced `payload`. `None` today.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EncScheme {
        /// No encryption; `payload` is the plain encoded `PeerFrame`.
        None,
        /// Reserved: Signal-style Double Ratchet, keyed by X3DH at pairing.
        SignalDoubleRatchet,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FrameEnvelope {
        pub v: u8,
        pub enc: EncScheme,
        #[serde(with = "base64_payload")]
        pub payload: Vec<u8>,
    }

    /// Base64, not serde_json's array of numbers: about 1.33 bytes for each
    /// byte instead of about 4, which matters on Bluetooth's small GATT
    /// fragments.
    mod base64_payload {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(&STANDARD.encode(value))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
            let encoded = String::deserialize(deserializer)?;
            STANDARD.decode(&encoded).map_err(serde::de::Error::custom)
        }
    }

    impl FrameEnvelope {
        pub fn new(enc: EncScheme, payload: Vec<u8>) -> Self {
            Self { v: ENVELOPE_VERSION, enc, payload }
        }
    }
}

/// The place for end-to-end encryption: a `SecureChannel` sits between the
/// codec and the `DataLink`. Today it is plain (`PlaintextChannel`).
pub mod encryption {
    use serde::{Deserialize, Serialize};

    use super::envelope::EncScheme;

    /// Reserved slot on `PairComplete` for a later key agreement. Always
    /// `None` on the wire today.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct KeyMaterial {
        pub scheme: EncScheme,
        pub identity_key: String,
        pub signed_prekey: String,
    }

    pub trait SecureChannel: Send + Sync {
        fn scheme(&self) -> EncScheme;
        fn encrypt(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, String>;
        fn decrypt(&self, ciphertext: Vec<u8>) -> Result<Vec<u8>, String>;
    }

    /// No encryption: the bytes go as they are.
    pub struct PlaintextChannel;

    impl SecureChannel for PlaintextChannel {
        fn scheme(&self) -> EncScheme {
            EncScheme::None
        }

        fn encrypt(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, String> {
            Ok(plaintext)
        }

        fn decrypt(&self, ciphertext: Vec<u8>) -> Result<Vec<u8>, String> {
            Ok(ciphertext)
        }
    }
}

pub mod iroh_link {
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

/// How long a connection may hear nothing before it ends. A held session
/// is green only while its connection lives, so this is how long a peer
/// whose radio went away (it cannot send a close) still shows green. iroh's
/// keep-alive is every 5 s, so 15 s is three missed heartbeats -- the same
/// as iroh's own path idle timeout. QUIC's default is 30 s.
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

/// The QUIC settings of both channels' endpoints.
pub fn transport_config() -> iroh::endpoint::QuicTransportConfig {
    iroh::endpoint::QuicTransportConfig::builder()
        .max_idle_timeout(CONNECTION_IDLE_TIMEOUT.try_into().ok())
        .build()
}

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
}
