//! The network transport: WebSocket `Link`s over TCP. Peer discovery for
//! this adapter is the existing mDNS/UDP presence worker
//! (`device_connection::runtime`) — `DeviceConnectionState::list_presenced_peers`
//! is this adapter's candidate list; there is no separate discovery step
//! here because that worker already runs continuously and is the thing the
//! rest of `device_connection` (add-device UI, etc.) also depends on.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, connect_async, MaybeTlsStream, WebSocketStream};

use crate::services::device_connection::DeviceConnectionState;
use crate::services::space_sync::session;
use crate::services::transport::{BoxDialFuture, Link, Transport, TransportKind};

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

pub struct TcpWsLink {
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

impl TcpWsLink {
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
impl Link for TcpWsLink {
    fn kind(&self) -> TransportKind {
        TransportKind::TcpWs
    }

    fn peer_addr(&self) -> Option<String> {
        self.peer_addr.clone()
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        // Text, not Binary: `device_connection::commands::send_pair_ws` sends
        // the one-shot pre-auth pairing frames (PairRequest/Accept/Complete)
        // over a raw tungstenite client, independent of this Link — it must
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

pub async fn dial(addr: IpAddr, port: u16) -> Result<Box<dyn Link>, String> {
    let url = ws_url(addr, port);
    let (ws, _) = connect_async(&url)
        .await
        .map_err(|err| format!("connect {url} failed: {err}"))?;
    Ok(Box::new(TcpWsLink::new(ws)))
}

/// `Transport` implementation for the network adapter. The production dial
/// loop (`spawn_dial_loop`) calls `dial()` directly rather than through this
/// trait object — there is no runtime plugin registry for two adapters —
/// but this impl proves the port is genuinely adapter-agnostic: both
/// `TcpWsTransport` and `transport::sim::SimTransport` satisfy the same
/// `Transport` trait, exercised together in `transport::tests`.
#[allow(dead_code)]
pub struct TcpWsTransport;

#[async_trait]
impl Transport for TcpWsTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::TcpWs
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
/// transport-neutral gate (`session::run_peer_gate`). `ui-plane`/`test`
/// only — see `session::run_peer_gate`'s doc comment.
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
            eprintln!("[transport][tcp_ws] failed to bind :{port}: {err}");
            return;
        }
    };
    eprintln!("[transport][tcp_ws] listening on :{port}");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!("[transport][tcp_ws] connection from {addr}");
                let state = state.clone();
                let db_path = db_path.clone();
                let peer_addr = Some(addr.ip().to_string());
                tokio::spawn(async move {
                    match accept_async(stream).await {
                        Ok(ws) => {
                            let link: Box<dyn Link> = Box::new(TcpWsLink::new_plain(ws, peer_addr));
                            session::run_peer_gate(link, state, db_path).await;
                        }
                        Err(err) => eprintln!("[transport][tcp_ws] WS handshake failed: {err}"),
                    }
                });
            }
            Err(err) => eprintln!("[transport][tcp_ws] accept error: {err}"),
        }
    }
}

/// Call from `space_sync_tick`: ensure an outbound session exists for every
/// paired, presenced peer where `self.device_id < peer.device_id`
/// (deterministic dialer rule) and no session on *this* transport is
/// active yet. ADR-0003 revision: no longer withdraws just because a
/// session already exists on Bluetooth, or because the peer is pinned to
/// Bluetooth -- both transports dial and stay connected independently now,
/// regardless of the pin (the pin only decides which connected transport
/// is primary, see `DeviceConnectionState::recompute_primary_locked`). A P1
/// review finding on this same revision: this is exactly why an in-flight
/// guard is now required here too (mirroring `ble::spawn_dial_loop`'s own,
/// pre-existing one) -- a peer that's presenced but whose WebSocket port is
/// permanently unreachable used to have Network's dial loop stand down for
/// good the instant *any* transport connected (the old any-transport
/// `has_session` check); now Network keeps trying indefinitely on its own
/// merits, so without this guard every tick would spawn *another*
/// concurrent `dial_with_backoff` retry loop for the same peer on top of
/// the ones already running.
pub fn spawn_dial_loop(
    state: &DeviceConnectionState,
    db_path: PathBuf,
    paired_peer_ids: &HashSet<String>,
) {
    let my_id = state.identity.device_id.clone();
    let peers = state.list_presenced_peers();

    for (peer_id, _addr, _ws_port) in peers {
        if !should_dial_peer(
            my_id.as_str(),
            peer_id.as_str(),
            paired_peer_ids,
            state.has_session_on(&peer_id, TransportKind::TcpWs),
        ) {
            continue;
        }
        if is_backing_off(&dial_backoff_until().lock().unwrap(), &peer_id, Instant::now()) {
            continue;
        }
        if !in_flight_dials().lock().unwrap().insert(peer_id.clone()) {
            continue;
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id = peer_id.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id.clone()).await;
            in_flight_dials().lock().unwrap().remove(&peer_id);
        });
    }
}

/// Peers with a `dial_with_backoff` task currently running. Mirrors
/// `ble::spawn_dial_loop`'s own guard of the same name/shape.
fn in_flight_dials() -> &'static std::sync::Mutex<HashSet<String>> {
    static IN_FLIGHT: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> = std::sync::OnceLock::new();
    IN_FLIGHT.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// Peers whose Network dial should not be retried before this instant.
/// Mirrors `ble::dial_backoff_until`/`DIAL_BACKOFF`/`is_backing_off` --
/// same P1 finding, same fix, other transport: a pre-v3 peer that already
/// has a Bluetooth session rejects our now-unconditional Network dial via
/// its own sticky-single-session code, and without this, the very next
/// `space_sync_tick` would spawn a fresh attempt against it immediately,
/// cycling connect/reject/disconnect indefinitely whenever Network becomes
/// available after Bluetooth.
fn dial_backoff_until() -> &'static std::sync::Mutex<HashMap<String, Instant>> {
    static BACKOFF: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    BACKOFF.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

const DIAL_BACKOFF: Duration = Duration::from_secs(60);

/// See `ble::is_backing_off`'s own doc comment for why this is split out
/// as a pure, directly-unit-testable function.
fn is_backing_off(backoff: &HashMap<String, Instant>, peer_id: &str, now: Instant) -> bool {
    backoff.get(peer_id).is_some_and(|until| now < *until)
}

fn should_dial_peer(
    my_id: &str,
    peer_id: &str,
    paired_peer_ids: &HashSet<String>,
    has_session: bool,
) -> bool {
    paired_peer_ids.contains(peer_id) && my_id < peer_id && !has_session
}

/// `pub(crate)`, not private: `transport::tests` exercises this directly
/// (bypassing `spawn_dial_loop`'s presence-worker plumbing) to prove its
/// error-handling distinguishes a genuine rejection from a transport
/// preference mismatch -- see the regression test for the P1 finding this
/// guards against.
///
/// Re-reads the peer's current `(addr, ws_port)` from presence on *every*
/// retry, not just once at spawn time -- a P1 review finding: with the
/// in-flight guard above now suppressing `spawn_dial_loop` from ever
/// re-spawning this peer while a task is already running for it, a stale
/// captured endpoint would otherwise keep this loop dialing an address the
/// peer stopped advertising indefinitely, even after discovery already
/// has the peer's real, current one -- exactly what `ble::dial_with_backoff`
/// already avoids by re-checking `is_still_bluetooth_eligible` (which reads
/// the current address) on every iteration.
pub(crate) async fn dial_with_backoff(
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_id: String,
) {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);

    loop {
        if state.has_session_on(&peer_id, TransportKind::TcpWs) {
            return;
        }
        let Some((_, addr, ws_port)) = state
            .list_presenced_peers()
            .into_iter()
            .find(|(id, _, _)| *id == peer_id)
        else {
            return; // no longer presenced
        };

        let Ok(target_addr) = addr.parse::<IpAddr>() else {
            eprintln!("[transport][tcp_ws] invalid peer addr '{addr}'");
            return;
        };

        match dial(target_addr, ws_port).await {
            Ok(mut link) => {
                match session::perform_client_auth(
                    link.as_mut(),
                    &state.identity.device_id,
                    &peer_id,
                )
                .await
                {
                    Ok(peer_protocol_version) => {
                        eprintln!("[transport][tcp_ws] auth OK with {peer_id}");
                        let (tx, rx) = tokio::sync::mpsc::channel(64);
                        if state.try_claim_session(&peer_id, TransportKind::TcpWs, tx, &db_path) {
                            session::run_session(
                                link,
                                rx,
                                state.clone(),
                                db_path.clone(),
                                peer_id.clone(),
                                peer_protocol_version,
                            )
                            .await;
                            eprintln!("[transport][tcp_ws] session with {peer_id} ended");
                        }
                        delay = Duration::from_secs(1);
                    }
                    Err(err) => {
                        eprintln!("[transport][tcp_ws] auth with {peer_id} failed: {err}");
                        if err.starts_with("auth rejected") {
                            // Not just "don't retry within this task" --
                            // `spawn_dial_loop` would otherwise spawn a
                            // fresh one on the very next tick regardless.
                            // See `dial_backoff_until`'s own doc comment.
                            dial_backoff_until()
                                .lock()
                                .unwrap()
                                .insert(peer_id.clone(), Instant::now() + DIAL_BACKOFF);
                            return; // not paired; don't retry
                        }
                    }
                }
            }
            Err(err) => {
                eprintln!("[transport][tcp_ws] connect to {peer_id} failed: {err}");
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn should_dial_only_paired_peers_where_local_id_wins_dialer_rule() {
        let paired = HashSet::from(["peer-b".to_string()]);

        assert!(should_dial_peer("local-a", "peer-b", &paired, false));
        assert!(!should_dial_peer("local-a", "peer-c", &paired, false));
        assert!(!should_dial_peer("peer-z", "peer-b", &paired, false));
        assert!(!should_dial_peer("local-a", "peer-b", &paired, true));
    }

    /// Regression test for a P1 review finding: a terminal auth rejection
    /// (e.g. a pre-v3 peer's own sticky-single-session code rejecting our
    /// now-unconditional Network dial while it already has a Bluetooth
    /// session with us) must suppress `spawn_dial_loop` from immediately
    /// spawning a fresh attempt against that peer on the very next tick.
    /// Mirrors `ble::dial_backoff_suppresses_a_peer_until_its_deadline_passes`.
    #[test]
    fn dial_backoff_suppresses_a_peer_until_its_deadline_passes() {
        let mut backoff = HashMap::new();
        let now = Instant::now();
        assert!(!is_backing_off(&backoff, "peer-a", now), "no entry yet -- must not back off");

        backoff.insert("peer-a".to_string(), now + Duration::from_secs(60));
        assert!(
            is_backing_off(&backoff, "peer-a", now),
            "within the backoff window -- must skip"
        );
        assert!(
            !is_backing_off(&backoff, "peer-b", now),
            "a different peer's entry must not affect this one"
        );
        assert!(
            !is_backing_off(&backoff, "peer-a", now + Duration::from_secs(61)),
            "once the deadline passes, the peer is eligible again"
        );
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
