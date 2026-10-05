//! The Network channel over iroh (ADR-0009 D2, D9). Each device has one
//! Network endpoint: iroh's IP transport only, relays off (D10), bound to
//! `DeviceConnectionState::space_sync_ws_port` over UDP. One QUIC connection
//! carries one exchange or one pairing frame (`iroh_link::IrohDataLink`).
//!
//! Peers are still found by Fini's own mDNS/UDP presence worker
//! (`pairing::runtime`), which supplies the address; iroh is told where to
//! connect rather than looking anything up. The key to dial is the one
//! pinned for the pair, or for a pair request the one the peer's beacon
//! announces; TLS proves it either way.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
#[cfg(any(feature = "ui-plane", test))]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iroh::endpoint::{presets, IncomingAddr};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, TransportAddr};

use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::DeviceConnectionState;
use crate::services::communication::sync::session;
use crate::services::communication::channel::iroh_link::{IrohDataLink, ALPN};
use crate::services::communication::channel::{send_frame, ChannelKind, DataLink};
use crate::services::communication::sync::types::PeerFrame;

/// How long a dial may take before it counts as failed. A peer on the same
/// network answers in milliseconds; one that went away never does.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// This state's Network endpoint, bound on first use to
/// `space_sync_ws_port`. A second process for the same device (the CLI next
/// to a running app) finds the port taken and binds any free one: it only
/// dials out.
pub(crate) async fn endpoint(state: &DeviceConnectionState) -> Result<Endpoint, String> {
    bind(state, state.space_sync_ws_port).await
}

async fn bind(state: &DeviceConnectionState, port: u16) -> Result<Endpoint, String> {
    state
        .network_endpoint
        .get_or_try_init(|| async {
            match bind_on(state, port).await {
                Ok(endpoint) => Ok(endpoint),
                Err(err) if port != 0 => {
                    log::warn!("[transport][network] port {port} unavailable ({err}); binding any free port");
                    bind_on(state, 0).await
                }
                Err(err) => Err(err),
            }
        })
        .await
        .cloned()
}

async fn bind_on(state: &DeviceConnectionState, port: u16) -> Result<Endpoint, String> {
    Endpoint::builder(presets::Minimal)
        .secret_key(state.secret_key.clone())
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .clear_ip_transports()
        .bind_addr(SocketAddr::from(([0, 0, 0, 0], port)))
        .map_err(|err| err.to_string())?
        .bind()
        .await
        .map_err(|err| format!("binding the network endpoint failed: {err}"))
}

/// The UDP port the Network endpoint is bound to.
#[cfg(test)]
pub(crate) fn bound_port(endpoint: &Endpoint) -> u16 {
    endpoint
        .bound_sockets()
        .first()
        .map(|addr| addr.port())
        .expect("the network endpoint has a socket")
}

/// Connects to the device holding `peer_key` at `ip:port`.
pub async fn dial(
    state: &DeviceConnectionState, peer_key: &str, ip: IpAddr, port: u16,
) -> Result<Box<dyn DataLink>, String> {
    Ok(Box::new(dial_link(state, peer_key, ip, port).await?))
}

async fn dial_link(
    state: &DeviceConnectionState, peer_key: &str, ip: IpAddr, port: u16,
) -> Result<IrohDataLink, String> {
    let endpoint = endpoint(state).await?;
    let id: EndpointId = peer_key
        .parse()
        .map_err(|err| format!("invalid peer key '{peer_key}': {err}"))?;
    let addr = EndpointAddr::from_parts(id, [TransportAddr::Ip(SocketAddr::new(ip, port))]);
    let connection = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(addr, ALPN))
        .await
        .map_err(|_| format!("connect to {ip}:{port} timed out"))?
        .map_err(|err| format!("connect to {ip}:{port} failed: {err}"))?;
    IrohDataLink::open(ChannelKind::Network, connection, Some(ip.to_string())).await
}

/// Sends one pre-auth frame (a pair request, accept or completion) and lets
/// the connection close once the peer has it.
pub async fn send_one_frame(
    state: &DeviceConnectionState, peer_key: &str, ip: IpAddr, port: u16, frame: &PeerFrame,
) -> Result<(), String> {
    let mut link = dial_link(state, peer_key, ip, port).await?;
    send_frame(&mut link, frame).await?;
    link.finish().await;
    Ok(())
}

/// Run the Network channel's accept loop on this state's endpoint and hand
/// every connection to the shared gate
/// (`crate::services::communication::pairing::run_peer_gate`). `ui-plane`/
/// `test` only -- see `run_peer_gate`'s doc comment.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_server(state: DeviceConnectionState, db_path: PathBuf) {
    match endpoint(&state).await {
        Ok(endpoint) => {
            log::info!("[transport][network] listening on {:?}", endpoint.bound_sockets());
            serve(state, db_path, endpoint).await;
        }
        Err(err) => log::error!("[transport][network] {err}"),
    }
}

/// Serves this state's endpoint bound on `port`.
#[cfg(test)]
pub(crate) async fn run_server_on_port(state: DeviceConnectionState, db_path: PathBuf, port: u16) {
    let endpoint = bind(&state, port).await.expect("bind the network endpoint");
    serve(state, db_path, endpoint).await;
}

/// Binds this state's endpoint on a free port, serves it, and returns the
/// port.
#[cfg(test)]
pub(crate) async fn spawn_server_on_free_port(state: DeviceConnectionState, db_path: PathBuf) -> u16 {
    let endpoint = bind(&state, 0).await.expect("bind the network endpoint");
    let port = bound_port(&endpoint);
    tokio::spawn(serve(state, db_path, endpoint));
    port
}

#[cfg(any(feature = "ui-plane", test))]
async fn serve(state: DeviceConnectionState, db_path: PathBuf, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let peer_addr = match incoming.remote_addr() {
            IncomingAddr::Ip(addr) => Some(addr.ip().to_string()),
            _ => None,
        };
        let state = state.clone();
        let db_path = db_path.clone();
        tokio::spawn(async move {
            let connection = match incoming.accept() {
                Ok(accepting) => match accepting.await {
                    Ok(connection) => connection,
                    Err(err) => {
                        log::warn!("[transport][network] handshake failed: {err}");
                        return;
                    }
                },
                Err(err) => {
                    log::warn!("[transport][network] accept failed: {err}");
                    return;
                }
            };
            match IrohDataLink::accept(ChannelKind::Network, connection, peer_addr).await {
                Ok(link) => {
                    crate::services::communication::pairing::run_peer_gate(Box::new(link), state, db_path).await;
                }
                Err(err) => log::warn!("[transport][network] {err}"),
            }
        });
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
    if recently_failed(peer_id) {
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
            // Look again now, while the failure still counts, so the work
            // can go over Bluetooth meanwhile; and once the cooldown is over,
            // to try Network again.
            crate::services::communication::sync::commands::notify_sync_work_pending();
            crate::services::communication::sync::commands::notify_sync_work_pending_after(
                FAILURE_COOLDOWN,
            );
        }
        in_flight_exchanges().lock().unwrap().remove(&peer_id);
    });
}

/// Whether this device is dialing the peer for an exchange right now.
#[cfg(any(feature = "ui-plane", test))]
pub fn dialing(peer_id: &str) -> bool {
    in_flight_exchanges().lock().unwrap().contains(peer_id)
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

/// Whether the last exchange attempt with this peer failed within
/// `FAILURE_COOLDOWN`.
pub fn recently_failed(peer_id: &str) -> bool {
    is_cooling_down(&failure_cooldown().lock().unwrap(), peer_id, Instant::now())
}

/// See `ChannelService::forget_failures`; also used when a pair is removed.
pub fn forget_failures(peer_id: &str) {
    failure_cooldown().lock().unwrap().remove(peer_id);
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
        log::warn!("[transport][network] invalid peer addr '{addr}'");
        return false;
    };
    let Some(peer_key) = tokio::task::block_in_place(|| {
        crate::services::communication::pairing::pinned_key(&mut open_db_at_path(&db_path), peer_id)
    }) else {
        // ADR-0009 D8: a pair from before keys existed is paired again.
        log::warn!("[transport][network] {peer_id} has no key pinned; pair the devices again");
        return false;
    };
    let mut link = match dial(state, &peer_key, target_addr, ws_port).await {
        Ok(link) => link,
        Err(err) => {
            log::warn!("[transport][network] connect to {peer_id} failed: {err}");
            return false;
        }
    };
    let peer_protocol_version =
        match session::perform_client_auth(link.as_mut(), &state.identity.device_id, peer_id).await {
            Ok(version) => version,
            // An exchange with the peer is already running (its own dial,
            // or the one that won a crossing): the work goes through that,
            // and this was no failure.
            Err(err) if session::refused_for_running_exchange(&err) => {
                log::info!("[transport][network] {peer_id}: an exchange is already running ({err})");
                return true;
            }
            Err(err) => {
                log::warn!("[transport][network] {peer_id} refused the exchange: {err}");
                return false;
            }
        };
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    if state.try_claim_session(peer_id, ChannelKind::Network, tx) {
        log::info!("[transport][network] exchange with {peer_id}");
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
}
