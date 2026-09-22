//! How the Bluetooth channel connects on a machine with no Bluetooth: a raw
//! TCP connection to `127.0.0.1` with length-delimited framing, in place of
//! GATT. Selected by `radio::for_this_device`, which is the only thing that
//! decides between this and the real radio.
//!
//! It is not a mock. The link it hands back goes through the same `DataLink`
//! port, the same codec, the same gate and the same session loop as a real
//! one — so a test running over it exercises everything except the radio,
//! which is the part CI cannot have. What it proves is the behaviour that
//! matters most and is hardest to arrange: the network is unavailable, so
//! the other channel carries the traffic.
//!
//! Its links report `ChannelKind::Bluetooth`, because that is what they
//! are: the Bluetooth channel, connected a different way. It used to report
//! a kind of its own, which meant the Bluetooth switch did not apply to it
//! and every test needed an `AsBluetooth` wrapper to paper over the gap.
//!
//! No discovery step, unlike `tcp_ws` and `ble`: peers are configured
//! directly via `FINI_LOOPBACK_PEER_PORTS` (one port per actor in a test run,
//! positionally, mirroring `FINI_DISCOVERY_PEER_PORTS`). The dial loop tries
//! each configured port against each paired peer it has no session with, and
//! the gate's `peer_device_id` check rejects wrong guesses harmlessly.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::{TcpListener, TcpStream};

use crate::services::communication::pairing::DeviceConnectionState;
use crate::services::communication::sync::session;
use crate::services::communication::channel::codec::length_delimited;
use crate::services::communication::channel::{BoxDialFuture, DataLink, Transport, ChannelKind};

pub struct LoopbackDataLink {
    stream: TcpStream,
}

impl LoopbackDataLink {
    /// `pub(crate)`, not private: `channel::tests` constructs one
    /// directly from an accepted `TcpStream` to act as a controlled fake
    /// peer in the TCP-failure-reset regression test, without needing the
    /// full `run_server` accept loop.
    pub(crate) fn new(stream: TcpStream) -> Self {
        Self { stream }
    }
}

#[async_trait]
impl DataLink for LoopbackDataLink {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        length_delimited::write(&mut self.stream, &payload).await
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        match length_delimited::read(&mut self.stream).await {
            Ok(Some(payload)) => Some(Ok(payload)),
            Ok(None) => None,
            // Name the socket, with its port. `peer_addr()` above cannot:
            // pairing stores that value as the peer's observed address, so
            // it has to stay an address. Here the port is the whole point --
            // on loopback it is the only thing that says which connection,
            // and therefore which writer, this came from.
            Err(err) => {
                let from = self
                    .stream
                    .peer_addr()
                    .map(|addr| addr.to_string())
                    .unwrap_or_else(|_| "?".to_string());
                Some(Err(format!("{err} (from {from})")))
            }
        }
    }

    fn peer_addr(&self) -> Option<String> {
        self.stream.peer_addr().ok().map(|addr| addr.ip().to_string())
    }
}

/// Read `FINI_LOOPBACK_PORT`; `None` means the loopback radio is not configured
/// for this process (the default — zero cost for normal desktop usage).
pub fn configured_listen_port() -> Option<u16> {
    std::env::var("FINI_LOOPBACK_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
}

fn configured_peer_ports() -> Vec<u16> {
    std::env::var("FINI_LOOPBACK_PEER_PORTS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| item.trim().parse::<u16>().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn dial(port: u16) -> Result<Box<dyn DataLink>, String> {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|err| format!("loopback connect 127.0.0.1:{port} failed: {err}"))?;
    Ok(Box::new(LoopbackDataLink::new(stream)))
}

/// `Transport` implementation for the loopback radio — see the note on
/// `channel::tcp_ws::TcpWsTransport` for why production dial loops call
/// `dial()` directly rather than through this trait object.
#[allow(dead_code)]
pub struct LoopbackTransport;

#[async_trait]
impl Transport for LoopbackTransport {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn dial(&self, _peer_device_id: &str, _addr: &str, port: u16) -> BoxDialFuture {
        Box::pin(async move { dial(port).await })
    }
}

/// Start the loopback listener if `FINI_LOOPBACK_PORT` is configured; no-op
/// otherwise. Mirrors `channel::tcp_ws::run_server` but with raw framing.
/// `ui-plane`/`test` only — see `crate::services::communication::pairing::run_peer_gate`'s doc comment.
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
            eprintln!("[channel][loopback] failed to bind :{port}: {err}");
            return;
        }
    };
    eprintln!("[channel][loopback] listening on :{port}");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!("[channel][loopback] connection from {addr}");
                let link: Box<dyn DataLink> = Box::new(LoopbackDataLink::new(stream));
                let state = state.clone();
                let db_path = db_path.clone();
                tokio::spawn(crate::services::communication::pairing::run_peer_gate(link, state, db_path));
            }
            Err(err) => eprintln!("[channel][loopback] accept error: {err}"),
        }
    }
}

/// Dial loop: for every paired peer with no active session on the Bluetooth
/// channel, try each configured loopback peer port. No-op unless
/// `FINI_LOOPBACK_PEER_PORTS` is set -- see the module docs for when that
/// happens. Applies
/// `should_dial_fallback_peer`'s deterministic dialer rule, mirroring
/// `tcp_ws::spawn_dial_loop`/`should_dial_peer`. ADR-0003 revision: dials
/// unconditionally now, independent of Network's own state or the pin --
/// both transports stay connected regardless (see
/// `tcp_ws::spawn_dial_loop`'s own doc comment, including why the
/// in-flight guard below is required now too).
pub fn spawn_fallback_dial_loop(
    state: &DeviceConnectionState,
    db_path: PathBuf,
    paired_peer_ids: &HashSet<String>,
) {
    let candidate_ports = configured_peer_ports();
    if candidate_ports.is_empty() {
        return;
    }

    let my_id = state.identity.device_id.clone();

    for peer_id in paired_peer_ids {
        if !should_dial_fallback_peer(&my_id, peer_id) {
            continue;
        }
        if state.has_session_on(peer_id, ChannelKind::Bluetooth) {
            continue;
        }
        if !in_flight_dials().lock().unwrap().insert(peer_id.clone()) {
            continue;
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id = peer_id.clone();
        let ports = candidate_ports.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id.clone(), ports).await;
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

/// Deterministic dialer rule for the fallback role, mirroring
/// `tcp_ws::should_dial_peer`'s `self.device_id < peer.device_id`: exactly
/// one side of a pair ever attempts to dial. Without this, both peers can
/// dial each other in the same tick — each accepts the other's inbound
/// connection and claims a session on it, then each side's own outbound
/// dial loses that claim race (already claimed by the inbound) and drops
/// its outbound `DataLink`, which — since both ends of one TCP connection share
/// a socket — tears down the peer's just-claimed inbound session too,
/// leaving both disconnected until a tick happens not to race.
fn should_dial_fallback_peer(my_id: &str, peer_id: &str) -> bool {
    my_id < peer_id
}

pub(crate) async fn dial_with_backoff(
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_id: String,
    candidate_ports: Vec<u16>,
) {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(15);

    loop {
        if state.has_session_on(&peer_id, ChannelKind::Bluetooth) {
            return;
        }

        for port in &candidate_ports {
            let Ok(mut link) = dial(*port).await else {
                continue;
            };
            match session::perform_client_auth(link.as_mut(), &state.identity.device_id, &peer_id)
                .await
            {
                Ok(peer_protocol_version) => {
                    eprintln!("[channel][loopback] auth OK with {peer_id} via :{port}");
                    let (tx, rx) = tokio::sync::mpsc::channel(64);
                    if state.try_claim_session(&peer_id, ChannelKind::Bluetooth, tx, &db_path) {
                        session::run_session(
                            link,
                            rx,
                            state.clone(),
                            db_path.clone(),
                            peer_id.clone(),
                            peer_protocol_version,
                        )
                        .await;
                        eprintln!("[channel][loopback] session with {peer_id} ended");
                        return;
                    }
                    // Lost the claim race (e.g. the peer's inbound accept claimed a session on
                    // this same peer+transport first) — this auth'd link is now redundant, not a
                    // failure to retry from scratch. Fall through to backoff so the next loop
                    // iteration's has_session_on check (which should now see the winning session)
                    // short-circuits, instead of this task silently exiting and leaving nothing
                    // to notice if that session never actually materializes.
                    eprintln!("[channel][loopback] lost claim race with {peer_id} via :{port}");
                }
                Err(err) => {
                    // Was discarded entirely, which made every failure here
                    // look identical to "that port was a wrong guess" -- and
                    // this loop dials its own listener too, so wrong guesses
                    // are normal and hid the rest.
                    eprintln!("[channel][loopback] auth via :{port} failed: {err}");
                    continue;
                }
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
    fn should_dial_fallback_peer_only_from_the_lower_device_id() {
        assert!(should_dial_fallback_peer("local-a", "peer-b"));
        assert!(!should_dial_fallback_peer("peer-b", "local-a"));
        assert!(!should_dial_fallback_peer("same-id", "same-id"));
    }
}
