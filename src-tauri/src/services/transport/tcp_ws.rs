//! The network transport: WebSocket `Link`s over TCP. Peer discovery for
//! this adapter is the existing mDNS/UDP presence worker
//! (`device_connection::runtime`) — `DeviceConnectionState::list_presenced_peers`
//! is this adapter's candidate list; there is no separate discovery step
//! here because that worker already runs continuously and is the thing the
//! rest of `device_connection` (add-device UI, etc.) also depends on.

use std::collections::HashSet;
use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

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

pub struct TcpWsLink {
    sink: BoxedSink,
    source: BoxedSource,
    peer_addr: Option<String>,
}

impl TcpWsLink {
    fn new(ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        let (sink, source) = ws.split();
        Self {
            sink: Box::pin(sink),
            source: Box::pin(source),
            peer_addr: None,
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
            match self.source.next().await {
                Some(Ok(Message::Text(text))) => return Some(Ok(text.as_bytes().to_vec())),
                Some(Ok(Message::Close(_))) => return None,
                Some(Ok(_)) => continue,
                Some(Err(err)) => return Some(Err(err.to_string())),
                None => return None,
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
/// (deterministic dialer rule) and no session (on any transport) is active.
pub fn spawn_dial_loop(
    state: &DeviceConnectionState,
    db_path: PathBuf,
    paired_peer_ids: &HashSet<String>,
) {
    let my_id = state.identity.device_id.clone();
    let peers = state.list_presenced_peers();

    for (peer_id, addr, ws_port) in peers {
        if !should_dial_peer(
            my_id.as_str(),
            peer_id.as_str(),
            paired_peer_ids,
            state.has_session(&peer_id),
        ) {
            continue;
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id_clone = peer_id.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id_clone, addr, ws_port).await;
        });
    }
}

fn should_dial_peer(
    my_id: &str,
    peer_id: &str,
    paired_peer_ids: &HashSet<String>,
    has_session: bool,
) -> bool {
    paired_peer_ids.contains(peer_id) && my_id < peer_id && !has_session
}

async fn dial_with_backoff(
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_id: String,
    addr: String,
    ws_port: u16,
) {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);

    loop {
        if state.has_session(&peer_id) {
            return;
        }
        let still_present = state
            .list_presenced_peers()
            .into_iter()
            .any(|(id, _, _)| id == peer_id);
        if !still_present {
            return;
        }

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
                    Ok(()) => {
                        eprintln!("[transport][tcp_ws] auth OK with {peer_id}");
                        let (tx, rx) = tokio::sync::mpsc::channel(64);
                        if state.try_claim_session(&peer_id, TransportKind::TcpWs, tx) {
                            session::run_session(
                                link,
                                rx,
                                state.clone(),
                                db_path.clone(),
                                peer_id.clone(),
                            )
                            .await;
                            eprintln!("[transport][tcp_ws] session with {peer_id} ended");
                        }
                        delay = Duration::from_secs(1);
                    }
                    Err(err) => {
                        eprintln!("[transport][tcp_ws] auth with {peer_id} failed: {err}");
                        if err.starts_with("auth rejected") {
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

    #[test]
    fn should_dial_only_paired_peers_where_local_id_wins_dialer_rule() {
        let paired = HashSet::from(["peer-b".to_string()]);

        assert!(should_dial_peer("local-a", "peer-b", &paired, false));
        assert!(!should_dial_peer("local-a", "peer-c", &paired, false));
        assert!(!should_dial_peer("peer-z", "peer-b", &paired, false));
        assert!(!should_dial_peer("local-a", "peer-b", &paired, true));
    }
}
