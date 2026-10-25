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

use serde::{Deserialize, Serialize};

pub use crate::models::channel::ChannelKind;
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
    use super::PeerFrame;

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

// ---- The data that the channels carry and give ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEventEnvelope {
    pub event_id: String,
    pub correlation_id: String,
    pub origin_device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub op_type: String,
    pub payload: Option<String>,
    pub updated_at: String,
    pub created_at: String,
}

impl SyncEventEnvelope {
    /// Whether this change wins over `existing`, a change to the same
    /// entity: the newer `updated_at` wins; on a tie the lower
    /// `origin_device_id`, then the lower `event_id`.
    pub fn wins_over(&self, existing: &SyncEventEnvelope) -> bool {
        match self.updated_at.cmp(&existing.updated_at) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => match self.origin_device_id.cmp(&existing.origin_device_id) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => self.event_id < existing.event_id,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSpaceDescriptor {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequestPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    #[serde(default)]
    pub from_discovery_port: Option<u16>,
    #[serde(default)]
    pub from_ws_port: Option<u16>,
    pub to_device_id: String,
    pub created_at: String,
    pub expires_at: String,
    /// The sender's iroh key. Self-reported, for pairing over a plain
    /// Bluetooth link that proves none; over the Network channel the key
    /// TLS proved wins (ADR-0009 D8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairAcceptPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub code: String,
    pub from_device_id: String,
    pub to_device_id: String,
    pub accepted_at: String,
    /// The sender's iroh key; see `PairRequestPayload::from_endpoint_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCompletePayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub to_device_id: String,
    pub paired_at: String,
    /// The completing peer's own Bluetooth address, if known, sent whatever
    /// channel carries this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluetooth_address: Option<String>,
    /// Reserved for a later key agreement. Unused today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_material: Option<crate::services::channel_service::encryption::KeyMaterial>,
    /// The sender's iroh key; see `PairRequestPayload::from_endpoint_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_endpoint_id: Option<String>,
}

/// A message of the Fini peer protocol: the frames before a session
/// (pairing, adding a channel, identifying a device; `PEER_ALPN`) and the
/// frames of a sync session (`SYNC_ALPN`).
///
/// A new session frame that an older peer must not receive changes the
/// version in `SYNC_ALPN`: two builds on different versions then share no
/// ALPN and do not connect. `PeerFrame::Unknown` only protects a build's
/// own decoding of frames it does not know.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PeerFrame {
    /// Starts pairing: a device in Add Device asks another to pair.
    #[serde(rename = "pair_start")]
    PairStart(PairRequestPayload),
    #[serde(rename = "pair_accept")]
    PairAccept(PairAcceptPayload),
    #[serde(rename = "pair_complete")]
    PairComplete(PairCompletePayload),
    #[serde(rename = "sync_event")]
    SyncEvent(SyncEventEnvelope),
    #[serde(rename = "ack")]
    Ack { event_id: String },
    #[serde(rename = "bootstrap_start")]
    BootstrapStart { space_id: String },
    #[serde(rename = "bootstrap_end")]
    BootstrapEnd { space_id: String, completed_at: String },
    #[serde(rename = "space_mapping_update")]
    SpaceMappingUpdate {
        mapped_space_ids: Vec<String>,
        custom_spaces: Vec<CustomSpaceDescriptor>,
        sent_at: String,
    },
    #[serde(rename = "space_sync_end")]
    SpaceSyncEnd { space_id: String, ended_at: String },
    /// This device's own Bluetooth address, sent once on a Network session
    /// by a device that can read it (Linux).
    #[serde(rename = "bluetooth_address_update")]
    BluetoothAddressUpdate { address: String },
    /// Before pairing, over a fresh Bluetooth link: asks the device for its
    /// id, name and iroh key, which an advertisement is too small to carry.
    /// Untrusted; carries the sender's own identity too.
    #[serde(rename = "identify_device")]
    IdentifyDevice { device_id: String, hostname: String, endpoint_id: String },
    /// Reply to `IdentifyDevice`, sent only while the receiver is in Add
    /// Device itself.
    #[serde(rename = "device_identified")]
    DeviceIdentified { device_id: String, hostname: String, endpoint_id: String },
    /// One half of a channel's init (ADR-0008 D1, D2), sent by a device that
    /// runs a setup for a paired device. Answered with `AddChannelAccept`
    /// only while the receiver runs its own setup for the sender on this
    /// channel, or the channel exists there.
    #[serde(rename = "add_channel")]
    AddChannel { device_id: String },
    /// Reply to `AddChannel`; carries the replier's own device id.
    #[serde(rename = "add_channel_accept")]
    AddChannelAccept { device_id: String },
    /// "I unlinked this channel on my side" (ADR-0008 D14). The receiver
    /// removes its own row for it.
    #[serde(rename = "channel_unlinked")]
    ChannelUnlinked { kind: ChannelKind },
    /// Any `type` tag this build does not know: skipped, so that a newer
    /// peer's frame does not end the session.
    #[serde(other)]
    Unknown,
}

impl PeerFrame {
    /// The frame's wire `type` alone, without its payload -- safe to log,
    /// where the frame itself may carry quest data.
    pub fn wire_type(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.get("type")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(event_id: &str, origin_device_id: &str, updated_at: &str) -> SyncEventEnvelope {
        SyncEventEnvelope {
            event_id: event_id.to_string(),
            correlation_id: "corr-1".to_string(),
            origin_device_id: origin_device_id.to_string(),
            entity_type: "quest".to_string(),
            entity_id: "q1".to_string(),
            space_id: "1".to_string(),
            op_type: "upsert".to_string(),
            payload: None,
            updated_at: updated_at.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn the_newer_change_wins_and_ties_go_to_the_lower_ids() {
        let t1 = "2026-03-01T00:00:00Z";
        let t2 = "2026-03-02T00:00:00Z";
        assert!(change("e1", "a", t2).wins_over(&change("e2", "b", t1)));
        assert!(!change("e1", "a", t1).wins_over(&change("e2", "b", t2)));
        assert!(change("e1", "aaa", t1).wins_over(&change("e2", "bbb", t1)));
        assert!(!change("e1", "bbb", t1).wins_over(&change("e2", "aaa", t1)));
        assert!(change("aaa", "d", t1).wins_over(&change("bbb", "d", t1)));
        assert!(!change("bbb", "d", t1).wins_over(&change("aaa", "d", t1)));
    }

    /// The log line for an unexpected first frame names the frame by its
    /// wire type and nothing else: a payload can carry quest data.
    #[test]
    fn wire_type_names_the_frame_without_its_payload() {
        let identify = PeerFrame::IdentifyDevice {
            device_id: "d".to_string(),
            hostname: "h".to_string(),
            endpoint_id: "k".to_string(),
        };
        assert_eq!(identify.wire_type(), "identify_device");
    }
}

/// A device that Add Device can list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub hostname: String,
    /// An IP address for Network, a Bluetooth address for Bluetooth.
    pub addr: String,
    pub discovery_port: u16,
    pub ws_port: Option<u16>,
    pub last_seen_at: String,
    /// The channel that heard the device.
    #[serde(default)]
    pub channel_kind: ChannelKind,
}

/// A problem on this device's channel: what the ⓘ popup on an orange row
/// tells. Each channel reports its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProblem {
    /// This platform has no Bluetooth support at all.
    BluetoothNotSupported,
    /// Bluetooth is off, missing, or does not work on this device.
    BluetoothUnavailable,
    /// This device cannot announce itself on the local network.
    NetworkUnavailable,
    /// This pair has no pinned key, so no channel of it can connect. Pairs
    /// made before ADR-0009 have none; they are paired again.
    PairKeyMissing,
}

