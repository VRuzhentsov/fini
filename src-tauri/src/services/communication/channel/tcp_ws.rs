//! The network transport: WebSocket `DataLink`s over TCP. Peer discovery for
//! this adapter is the existing mDNS/UDP presence worker
//! (`pairing::runtime`) — `DeviceConnectionState::list_presenced_peers`
//! is this adapter's candidate list; there is no separate discovery step
//! here because that worker already runs continuously and is the thing the
//! rest of `device_connection` (add-device UI, etc.) also depends on.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
#[cfg(any(feature = "ui-plane", test))]
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
#[cfg(any(feature = "ui-plane", test))]
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
#[cfg(any(feature = "ui-plane", test))]
use tokio_tungstenite::accept_async;

use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::DeviceConnectionState;
use crate::services::communication::sync::session;
use crate::services::communication::channel::{BoxDialFuture, DataLink, Transport, ChannelKind};

type BoxedSink = Pin<Box<dyn Sink<Message, Error = WsError> + Send>>;
type BoxedSource = Pin<Box<dyn Stream<Item = Result<Message, WsError>> + Send>>;

/// How often `recv()` sends a WebSocket-native `Ping` while otherwise idle.
/// See `docs/adr/0003-transport-liveness-unified-status-and-manual-switching.md`
/// Phase 1: this is the whole liveness mechanism for this transport,
/// self-contained here — `run_session` never knows it exists, it just sees
/// `recv()` eventually return `None` like any other dead link.
const PING_INTERVAL: Duration = Duration::from_secs(15);
/// Consecutive `PING_INTERVAL` ticks with no `Pong` in between before
/// `recv()` gives up and reports the link dead (~45s: 15s × 3).
const PING_MISS_LIMIT: u32 = 3;

pub struct TcpWsDataLink {
    sink: BoxedSink,
    source: BoxedSource,
    peer_addr: Option<String>,
    ping_interval: tokio::time::Interval,
    /// How many `PING_INTERVAL` ticks have fired since the last `Pong`
    /// (or since the link was created, if none has arrived yet). Reset to
    /// 0 by any inbound `Message::Pong`; `recv()` declares the link dead
    /// once this reaches `PING_MISS_LIMIT`.
    missed_pongs: u32,
}

impl TcpWsDataLink {
    fn new(ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        let (sink, source) = ws.split();
        Self {
            sink: Box::pin(sink),
            source: Box::pin(source),
            peer_addr: None,
            ping_interval: tokio::time::interval(PING_INTERVAL),
            missed_pongs: 0,
        }
    }

    /// `peer_addr` is captured by the caller from the raw `TcpStream`
    /// *before* the WS upgrade (`accept_async` takes ownership of the
    /// stream) — matches how the original `ws_server::handle_connection`
    /// captured it, and is what makes `PairAccept`/`PairComplete` able to
    /// address their reply back to the pre-auth `PairRequest` sender.
    #[cfg(any(feature = "ui-plane", test))]
    fn new_plain(ws: WebSocketStream<TcpStream>, peer_addr: Option<String>) -> Self {
        let (sink, source) = ws.split();
        Self {
            sink: Box::pin(sink),
            source: Box::pin(source),
            peer_addr,
            ping_interval: tokio::time::interval(PING_INTERVAL),
            missed_pongs: 0,
        }
    }
}

#[async_trait]
impl DataLink for TcpWsDataLink {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Network
    }

    fn peer_addr(&self) -> Option<String> {
        self.peer_addr.clone()
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        // Text, not Binary: `pairing::commands::send_pair_ws` sends
        // the one-shot pre-auth pairing frames (PairRequest/Accept/Complete)
        // over a raw tungstenite client, independent of this DataLink — it must
        // stay wire-compatible with whatever this side reads. `codec::encode_frame`
        // always produces valid UTF-8 JSON, so this is lossless.
        let text = String::from_utf8(payload)
            .map_err(|err| format!("non-utf8 frame payload: {err}"))?;
        self.sink
            .send(Message::Text(text.into()))
            .await
            .map_err(|err| err.to_string())
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        loop {
            tokio::select! {
                item = self.source.next() => {
                    match item {
                        Some(Ok(Message::Text(text))) => return Some(Ok(text.as_bytes().to_vec())),
                        Some(Ok(Message::Close(_))) => return None,
                        // tungstenite auto-replies to an inbound Ping on its
                        // own (queues a Pong for the next write) -- nothing
                        // to do here beyond letting it pass through. Only an
                        // inbound Pong is this side's business: it's the
                        // liveness signal the interval arm below is waiting
                        // for.
                        Some(Ok(Message::Pong(_))) => {
                            self.missed_pongs = 0;
                            continue;
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(err)) => return Some(Err(err.to_string())),
                        None => return None,
                    }
                }
                _ = self.ping_interval.tick() => {
                    if self.missed_pongs >= PING_MISS_LIMIT {
                        return None;
                    }
                    if self.sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        return None;
                    }
                    self.missed_pongs += 1;
                }
            }
        }
    }
}

fn ws_url(addr: IpAddr, port: u16) -> String {
    match addr {
        IpAddr::V4(_) => format!("ws://{addr}:{port}"),
        IpAddr::V6(_) => format!("ws://[{addr}]:{port}"),
    }
}

pub async fn dial(addr: IpAddr, port: u16) -> Result<Box<dyn DataLink>, String> {
    let url = ws_url(addr, port);
    let (ws, _) = connect_async(&url)
        .await
        .map_err(|err| format!("connect {url} failed: {err}"))?;
    Ok(Box::new(TcpWsDataLink::new(ws)))
}

/// `Transport` implementation for the network adapter. The production dial
/// loop (`spawn_dial_loop`) calls `dial()` directly rather than through this
/// trait object — there is no runtime plugin registry for two adapters —
/// but this impl proves the port is genuinely adapter-agnostic: both
/// `TcpWsTransport` and `channel::loopback::LoopbackTransport` satisfy the same
/// `Transport` trait, exercised together in `channel::tests`.
#[allow(dead_code)]
pub struct TcpWsTransport;

#[async_trait]
impl Transport for TcpWsTransport {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Network
    }

    fn dial(&self, _peer_device_id: &str, addr: &str, port: u16) -> BoxDialFuture {
        let addr = addr.to_string();
        Box::pin(async move {
            let ip: IpAddr = addr
                .parse()
                .map_err(|err| format!("invalid network address '{addr}': {err}"))?;
            dial(ip, port).await
        })
    }
}

/// Run the network transport's server loop: bind `state.space_sync_ws_port`,
/// accept connections, WS-upgrade each, and hand off to the shared
/// transport-neutral gate (`crate::services::communication::pairing::run_peer_gate`). `ui-plane`/`test`
/// only — see `crate::services::communication::pairing::run_peer_gate`'s doc comment.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_server(state: DeviceConnectionState, db_path: PathBuf) {
    let port = state.space_sync_ws_port;
    run_server_on_port(state, db_path, port).await;
}

#[cfg(any(feature = "ui-plane", test))]
pub(crate) async fn run_server_on_port(
    state: DeviceConnectionState,
    db_path: PathBuf,
    port: u16,
) {
    let listener = match TcpListener::bind(format!("0.0.0.0:{port}")).await {
        Ok(l) => l,
        Err(err) => {
            log::error!("[transport][tcp_ws] failed to bind :{port}: {err}");
            return;
        }
    };
    log::info!("[transport][tcp_ws] listening on :{port}");
    serve(state, db_path, listener).await;
}

/// Binds a free port and serves it, returning the port. Binding before
/// handing the port back is the point: picking a free port, releasing it
/// and binding it later let a test running in parallel take it first.
#[cfg(test)]
pub(crate) async fn spawn_server_on_free_port(state: DeviceConnectionState, db_path: PathBuf) -> u16 {
    let listener = TcpListener::bind("0.0.0.0:0").await.expect("bind a free port");
    let port = listener.local_addr().expect("bound address").port();
    tokio::spawn(serve(state, db_path, listener));
    port
}

#[cfg(any(feature = "ui-plane", test))]
async fn serve(state: DeviceConnectionState, db_path: PathBuf, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                log::info!("[transport][tcp_ws] connection from {addr}");
                let state = state.clone();
                let db_path = db_path.clone();
                let peer_addr = Some(addr.ip().to_string());
                tokio::spawn(async move {
                    match accept_async(stream).await {
                        Ok(ws) => {
                            let link: Box<dyn DataLink> = Box::new(TcpWsDataLink::new_plain(ws, peer_addr));
                            crate::services::communication::pairing::run_peer_gate(link, state, db_path).await;
                        }
                        Err(err) => log::warn!("[transport][tcp_ws] WS handshake failed: {err}"),
                    }
                });
            }
            Err(err) => log::warn!("[transport][tcp_ws] accept error: {err}"),
        }
    }
}

/// Start an exchange with this peer over the Network channel unless one is
/// running or being attempted (ADR-0008 D10). Called when there is work for
/// the peer; a no-op while the peer is not present on the network -- its
/// presence beacon arriving wakes the keeper, which asks again.
pub fn start_exchange(state: &DeviceConnectionState, peer_id: &str) {
    if state.has_session_on(peer_id, ChannelKind::Network) || !state.network_peer_available(peer_id) {
        return;
    }
    if is_cooling_down(&failure_cooldown().lock().unwrap(), peer_id, Instant::now()) {
        return;
    }
    if !in_flight_exchanges().lock().unwrap().insert(peer_id.to_string()) {
        return;
    }
    let state = state.clone();
    let peer_id = peer_id.to_string();
    tauri::async_runtime::spawn(async move {
        if !exchange_with(&state, &peer_id).await {
            failure_cooldown()
                .lock()
                .unwrap()
                .insert(peer_id.clone(), Instant::now() + FAILURE_COOLDOWN);
        }
        in_flight_exchanges().lock().unwrap().remove(&peer_id);
    });
}

/// Peers with an exchange attempt in flight -- one at a time per peer.
fn in_flight_exchanges() -> &'static std::sync::Mutex<HashSet<String>> {
    static IN_FLIGHT: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> = std::sync::OnceLock::new();
    IN_FLIGHT.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// After a failed attempt at a present peer (refused, unreachable port),
/// wait this long before the next, so a peer that cannot take an exchange
/// does not turn every wake into another connection.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);

fn failure_cooldown() -> &'static std::sync::Mutex<HashMap<String, Instant>> {
    static COOLDOWN: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    COOLDOWN.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn is_cooling_down(cooldown: &HashMap<String, Instant>, peer_id: &str, now: Instant) -> bool {
    cooldown.get(peer_id).is_some_and(|until| now < *until)
}

/// Whether the Network channel is still switched on for this pair.
fn is_still_network_eligible(db_path: &std::path::Path, peer_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        crate::services::communication::pairing::channels::is_enabled(
            &mut conn,
            peer_id,
            crate::services::communication::pairing::ChannelKind::Network,
        )
    })
}

/// One exchange: connect to where the peer's presence beacon was heard,
/// authenticate, and run the exchange to its idle end. `false` if it could
/// not be started at all.
async fn exchange_with(state: &DeviceConnectionState, peer_id: &str) -> bool {
    let db_path = state.db_path.clone();
    if !is_still_network_eligible(&db_path, peer_id) {
        return true;
    }
    let Some((addr, ws_port)) = state.network_presence_address(peer_id) else {
        return true;
    };
    let Ok(target_addr) = addr.parse::<IpAddr>() else {
        log::warn!("[transport][tcp_ws] invalid peer addr '{addr}'");
        return false;
    };
    let mut link = match dial(target_addr, ws_port).await {
        Ok(link) => link,
        Err(err) => {
            log::warn!("[transport][tcp_ws] connect to {peer_id} failed: {err}");
            return false;
        }
    };
    let peer_protocol_version =
        match session::perform_client_auth(link.as_mut(), &state.identity.device_id, peer_id).await {
            Ok(version) => version,
            Err(err) => {
                log::warn!("[transport][tcp_ws] {peer_id} refused the exchange: {err}");
                return false;
            }
        };
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    if state.try_claim_session(peer_id, ChannelKind::Network, tx) {
        log::info!("[transport][tcp_ws] exchange with {peer_id}");
        session::run_session(
            link,
            rx,
            state.clone(),
            db_path,
            peer_id.to_string(),
            peer_protocol_version,
            session::EXCHANGE_IDLE,
        )
        .await;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn a_failed_attempt_cools_down_only_that_peer() {
        let mut cooldown = HashMap::new();
        let now = Instant::now();
        assert!(!is_cooling_down(&cooldown, "peer-a", now));

        cooldown.insert("peer-a".to_string(), now + FAILURE_COOLDOWN);
        assert!(is_cooling_down(&cooldown, "peer-a", now));
        assert!(!is_cooling_down(&cooldown, "peer-b", now));
        assert!(!is_cooling_down(&cooldown, "peer-a", now + FAILURE_COOLDOWN + Duration::from_secs(1)));
    }

    async fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    /// Regression test for ADR-0003 Phase 1: a peer that completes the WS
    /// handshake and then goes genuinely silent (still holding the TCP
    /// connection open, never reading or writing anything else -- unlike
    /// `Message::Close`, which the pre-existing code already handled) must
    /// eventually be reported dead, not block `recv()` forever. Paused time
    /// lets this run in real time without an actual ~45s wait.
    #[tokio::test(start_paused = true)]
    async fn recv_reports_the_link_dead_once_pings_go_unanswered() {
        let port = free_port().await;
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            // Never read or write again -- a connected-but-unresponsive peer.
            std::future::pending::<()>().await;
        });

        let mut link = dial("127.0.0.1".parse().unwrap(), port).await.unwrap();

        match link.recv().await {
            None => {}
            other => panic!("expected the link to be reported dead, got {other:?}"),
        }
    }

    /// Sibling regression test: a peer that keeps answering (a real
    /// `tokio-tungstenite` client auto-replies `Pong` to every `Ping`, per
    /// RFC 6455 -- nothing peer-side needs to do deliberately) must *not*
    /// be declared dead just because multiple `PING_INTERVAL`s have quietly
    /// elapsed with no application data in between.
    #[tokio::test(start_paused = true)]
    async fn recv_keeps_a_responsive_link_alive_across_several_ping_intervals() {
        let port = free_port().await;
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            // Idle long enough for several PING_INTERVAL ticks to fire
            // (tungstenite auto-replies Pong to each Ping on its own),
            // then prove the connection is still genuinely usable.
            tokio::time::sleep(PING_INTERVAL * (PING_MISS_LIMIT + 2)).await;
            ws.send(Message::Text("still alive".into())).await.unwrap();
        });

        let mut link = dial("127.0.0.1".parse().unwrap(), port).await.unwrap();

        match link.recv().await {
            Some(Ok(bytes)) => assert_eq!(bytes, b"still alive"),
            other => panic!("expected the still-live link to deliver its message, got {other:?}"),
        }
    }
}

