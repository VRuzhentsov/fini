//! The Sim transport: a deterministic, radio-free adapter used by Rust
//! integration tests and Playwright E2E to prove transport selection,
//! Bluetooth-role fallback, and sticky handoff without real hardware. It
//! implements the same `Link` port as every other adapter (raw TCP +
//! length-delimited framing instead of a WebSocket upgrade), so exercising
//! it exercises the real gate/session code in `space_sync::session` — this
//! is a first-class transport, not a mock of one.
//!
//! Unlike `tcp_ws`, Sim has no autonomous discovery step: peers are
//! configured directly via `FINI_SIM_PEER_PORTS` (one port per actor in a
//! test run, positionally, mirroring `FINI_DISCOVERY_PEER_PORTS`). The dial
//! loop tries each configured port against each paired peer id it doesn't
//! yet have a session with; the gate's existing `peer_device_id` check
//! rejects wrong guesses harmlessly. The real Bluetooth adapter (PR B) will
//! have genuine per-peer discovery via OS-paired addresses and won't need
//! this guess-and-check.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::{TcpListener, TcpStream};

use crate::services::device_connection::DeviceConnectionState;
use crate::services::space_sync::session;
use crate::services::transport::codec::length_delimited;
use crate::services::transport::{BoxDialFuture, Link, Transport, TransportKind};

pub struct SimLink {
    stream: TcpStream,
}

impl SimLink {
    fn new(stream: TcpStream) -> Self {
        Self { stream }
    }
}

#[async_trait]
impl Link for SimLink {
    fn kind(&self) -> TransportKind {
        TransportKind::Sim
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        length_delimited::write(&mut self.stream, &payload).await
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        match length_delimited::read(&mut self.stream).await {
            Ok(Some(payload)) => Some(Ok(payload)),
            Ok(None) => None,
            Err(err) => Some(Err(err)),
        }
    }

    fn peer_addr(&self) -> Option<String> {
        self.stream.peer_addr().ok().map(|addr| addr.ip().to_string())
    }
}

/// Read `FINI_SIM_TRANSPORT_PORT`; `None` means the Sim adapter is disabled
/// for this process (the default — zero cost for normal desktop usage).
pub fn configured_listen_port() -> Option<u16> {
    std::env::var("FINI_SIM_TRANSPORT_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
}

fn configured_peer_ports() -> Vec<u16> {
    std::env::var("FINI_SIM_PEER_PORTS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| item.trim().parse::<u16>().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn dial(port: u16) -> Result<Box<dyn Link>, String> {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|err| format!("sim connect 127.0.0.1:{port} failed: {err}"))?;
    Ok(Box::new(SimLink::new(stream)))
}

/// `Transport` implementation for the Sim adapter — see the note on
/// `transport::tcp_ws::TcpWsTransport` for why production dial loops call
/// `dial()` directly rather than through this trait object.
#[allow(dead_code)]
pub struct SimTransport;

#[async_trait]
impl Transport for SimTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Sim
    }

    fn dial(&self, _peer_device_id: &str, _addr: &str, port: u16) -> BoxDialFuture {
        Box::pin(async move { dial(port).await })
    }
}

/// Start the Sim listener if `FINI_SIM_TRANSPORT_PORT` is configured; no-op
/// otherwise. Mirrors `transport::tcp_ws::run_server` but with raw framing.
/// `ui-plane`/`test` only — see `session::run_peer_gate`'s doc comment.
#[cfg(any(feature = "ui-plane", test))]
pub fn maybe_spawn_server(state: DeviceConnectionState, db_path: PathBuf) {
    let Some(port) = configured_listen_port() else {
        return;
    };
    tauri::async_runtime::spawn(run_server(state, db_path, port));
}

#[cfg(any(feature = "ui-plane", test))]
pub(crate) async fn run_server(state: DeviceConnectionState, db_path: PathBuf, port: u16) {
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(err) => {
            eprintln!("[transport][sim] failed to bind :{port}: {err}");
            return;
        }
    };
    eprintln!("[transport][sim] listening on :{port}");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!("[transport][sim] connection from {addr}");
                let link: Box<dyn Link> = Box::new(SimLink::new(stream));
                let state = state.clone();
                let db_path = db_path.clone();
                tokio::spawn(session::run_peer_gate(link, state, db_path));
            }
            Err(err) => eprintln!("[transport][sim] accept error: {err}"),
        }
    }
}

/// Fallback dial loop: for every paired peer with no active session and no
/// available network transport, try each configured Sim peer port. No-op
/// unless `FINI_SIM_PEER_PORTS` is set. Playing the "Bluetooth fallback"
/// role for this PR's test/E2E coverage — see module docs.
pub fn spawn_fallback_dial_loop(
    state: &DeviceConnectionState,
    db_path: PathBuf,
    paired_peer_ids: &HashSet<String>,
) {
    let candidate_ports = configured_peer_ports();
    if candidate_ports.is_empty() {
        return;
    }

    for peer_id in paired_peer_ids {
        let order = crate::services::transport::selection::select_dial_order(
            state.network_peer_available(peer_id),
            true,
        );
        let sim_is_selected = order.contains(&TransportKind::Sim);
        if state.has_session(peer_id) || !sim_is_selected {
            continue;
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id = peer_id.clone();
        let ports = candidate_ports.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id, ports).await;
        });
    }
}

async fn dial_with_backoff(
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_id: String,
    candidate_ports: Vec<u16>,
) {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(15);

    loop {
        if state.has_session(&peer_id) || state.network_peer_available(&peer_id) {
            return;
        }

        for port in &candidate_ports {
            let Ok(mut link) = dial(*port).await else {
                continue;
            };
            match session::perform_client_auth(link.as_mut(), &state.identity.device_id, &peer_id)
                .await
            {
                Ok(()) => {
                    eprintln!("[transport][sim] auth OK with {peer_id} via :{port}");
                    let (tx, rx) = tokio::sync::mpsc::channel(64);
                    if state.try_claim_session(&peer_id, TransportKind::Sim, tx) {
                        session::run_session(link, rx, state.clone(), db_path.clone(), peer_id.clone())
                            .await;
                        eprintln!("[transport][sim] session with {peer_id} ended");
                    }
                    return;
                }
                Err(_) => continue, // wrong-guess port, or peer not yet listening
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}
