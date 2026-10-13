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

use iroh::endpoint::presets;
#[cfg(any(feature = "ui-plane", test))]
use iroh::endpoint::IncomingAddr;
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

/// The test switch of the Network channel, for debug builds only. Off makes
/// the network look absent to Fini: no announcements go out, none come in,
/// and connections fail. The real network stays on, so that no test must
/// switch off the Wi-Fi of a person's device.
#[cfg(feature = "devtools")]
static NETWORK_SWITCH_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the test switch set the network to off. Always false in a build
/// without `devtools`.
pub(crate) fn network_switched_off() -> bool {
    #[cfg(feature = "devtools")]
    return NETWORK_SWITCH_OFF.load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(not(feature = "devtools"))]
    false
}

/// Sets the test switch of the Network channel; see `NETWORK_SWITCH_OFF`.
#[cfg(feature = "devtools")]
pub fn set_network_switch(on: bool) {
    log::info!("[transport][network] test switch: network {}", if on { "on" } else { "off" });
    NETWORK_SWITCH_OFF.store(!on, std::sync::atomic::Ordering::SeqCst);
}

fn switched_off_error() -> String {
    "the test switch set the network to off".to_string()
}

/// How long the Network server waits before trying its port again.
#[cfg(any(feature = "ui-plane", test))]
const REBIND_INTERVAL: Duration = Duration::from_secs(5);

/// This state's Network endpoint, bound on first use to
/// `space_sync_ws_port`. A second process for the same device (the CLI next
/// to a running app) finds the port taken and binds any free one: it only
/// dials out. A state that serves never does: presence announces
/// `space_sync_ws_port`, so it fails instead and is tried again.
pub(crate) async fn endpoint(state: &DeviceConnectionState) -> Result<Endpoint, String> {
    bind(state, state.space_sync_ws_port).await
}

async fn bind(state: &DeviceConnectionState, port: u16) -> Result<Endpoint, String> {
    let serves = state.serves_network.load(std::sync::atomic::Ordering::SeqCst);
    state
        .network_endpoint
        .get_or_try_init(|| async {
            match bind_on(state, port).await {
                Ok(endpoint) => Ok(endpoint),
                Err(err) if port != 0 && !serves => {
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
    if network_switched_off() {
        return Err(switched_off_error());
    }
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
    state.serves_network.store(true, std::sync::atomic::Ordering::SeqCst);
    let endpoint = loop {
        match endpoint(&state).await {
            Ok(endpoint) => break endpoint,
            Err(err) => {
                log::warn!("[transport][network] {err}; trying again in {REBIND_INTERVAL:?}");
                tokio::time::sleep(REBIND_INTERVAL).await;
            }
        }
    };
    log::info!("[transport][network] listening on {:?}", endpoint.bound_sockets());
    serve(state, db_path, endpoint).await;
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
        if network_switched_off() {
            drop(incoming);
            continue;
        }
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

/// The mDNS service name for Fini's Network channel. Only Fini endpoints use
/// it, so other iroh apps on the network are not heard.
#[cfg(any(feature = "ui-plane", test))]
const MDNS_SERVICE_NAME: &str = "fini";

/// The user data this device publishes with its iroh mDNS record: a version,
/// the Add Device flag, the device id and the name, as
/// `fini1;a=<0|1>;d=<device id>;n=<name>`. iroh limits it to 245 bytes, so the
/// name is cut to fit.
#[cfg(any(feature = "ui-plane", test))]
fn network_user_data(state: &DeviceConnectionState) -> Option<iroh::endpoint_info::UserData> {
    let add_device = state.is_add_mode_enabled();
    let mut name = state.identity.hostname.clone();
    let prefix = format!("fini1;a={};d={};n=", u8::from(add_device), state.identity.device_id);
    while prefix.len() + name.len() > 245 {
        name.pop();
    }
    format!("{prefix}{name}").parse().ok()
}

/// What another device's user data says: Add Device flag, device id, name.
#[cfg(any(feature = "ui-plane", test))]
fn parse_network_user_data(data: &str) -> Option<(bool, String, String)> {
    let rest = data.strip_prefix("fini1;a=")?;
    let (flag, rest) = rest.split_once(";d=")?;
    let (device_id, name) = rest.split_once(";n=")?;
    Some((flag == "1", device_id.to_string(), name.to_string()))
}

/// Finds other Fini devices on the network with iroh's own mDNS lookup
/// (`iroh-mdns-address-lookup`), and announces this one. What it hears goes
/// to the same records the rest of the Network channel reads: presence for
/// paired devices, and the Add Device list.
///
/// The test switch (debug builds) stops the lookup while the network is off
/// and starts a new one when it is on, which asks the network again at once.
#[cfg(any(feature = "ui-plane", test))]
pub(crate) async fn run_network_discovery(state: DeviceConnectionState) {
    use futures_util::StreamExt;
    use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};

    let endpoint = loop {
        match endpoint(&state).await {
            Ok(endpoint) => break endpoint,
            Err(err) => {
                log::warn!("[transport][network] discovery waits for the endpoint: {err}");
                tokio::time::sleep(REBIND_INTERVAL).await;
            }
        }
    };
    loop {
        while network_switched_off() {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let mdns = match MdnsAddressLookup::builder().service_name(MDNS_SERVICE_NAME).build(endpoint.id()) {
            Ok(mdns) => mdns,
            Err(err) => {
                log::warn!("[transport][network] mDNS lookup failed to start: {err}");
                tokio::time::sleep(REBIND_INTERVAL).await;
                continue;
            }
        };
        let Ok(lookup) = endpoint.address_lookup() else {
            log::warn!("[transport][network] the endpoint has no address lookup");
            return;
        };
        lookup.add(mdns.clone());
        let mut published = network_user_data(&state);
        endpoint.set_user_data_for_address_lookup(published.clone());
        log::info!("[transport][network] mDNS lookup started");

        let mut events = mdns.subscribe().await;
        // Which device each endpoint is, to forget it when it expires.
        let mut devices: HashMap<EndpointId, String> = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                event = events.next() => match event {
                    Some(DiscoveryEvent::Discovered { endpoint_info, .. }) => {
                        let endpoint_id = endpoint_info.endpoint_id;
                        let Some((add_device, device_id, name)) = endpoint_info
                            .data
                            .user_data()
                            .and_then(|data| parse_network_user_data(data.as_ref()))
                        else {
                            continue;
                        };
                        if device_id == state.identity.device_id {
                            continue;
                        }
                        let Some(addr) = endpoint_info.data.ip_addrs().next().copied() else {
                            continue;
                        };
                        devices.insert(endpoint_id, device_id.clone());
                        state.note_network_device(&device_id, &name, addr, &endpoint_id.to_string(), add_device);
                    }
                    Some(DiscoveryEvent::Expired { endpoint_id }) => {
                        if let Some(device_id) = devices.remove(&endpoint_id) {
                            state.forget_network_device(&device_id);
                        }
                    }
                    Some(_) => {}
                    None => break,
                },
                _ = tick.tick() => {
                    if network_switched_off() {
                        break;
                    }
                    // Add Device opened or closed: publish the new flag.
                    let wanted = network_user_data(&state);
                    if wanted != published {
                        endpoint.set_user_data_for_address_lookup(wanted.clone());
                        published = wanted;
                    }
                    state.prune_network_records();
                }
            }
        }
        lookup.clear();
        log::info!("[transport][network] mDNS lookup stopped");
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
    let attempts = &state.network_attempts;
    if attempts.recently_failed(peer_id) {
        return;
    }
    if !attempts.in_flight.lock().unwrap().insert(peer_id.to_string()) {
        return;
    }
    let state = state.clone();
    let peer_id = peer_id.to_string();
    tauri::async_runtime::spawn(async move {
        let attempts = &state.network_attempts;
        if !exchange_with(&state, &peer_id).await {
            attempts
                .cooldown
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
        attempts.in_flight.lock().unwrap().remove(&peer_id);
    });
}

/// After a failed attempt at a present peer (refused, unreachable port),
/// wait this long before the next, so a peer that cannot take an exchange
/// does not turn every wake into another connection.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);

/// This state's Network exchange attempts: which run now, and which peers
/// failed recently. One per `DeviceConnectionState`.
#[derive(Default)]
pub struct ExchangeAttempts {
    /// Peers with an exchange attempt in flight -- one at a time per peer.
    in_flight: std::sync::Mutex<HashSet<String>>,
    /// Until when each peer whose last attempt failed is left alone.
    cooldown: std::sync::Mutex<HashMap<String, Instant>>,
}

impl ExchangeAttempts {
    /// Whether this device is dialing the peer for an exchange right now.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn dialing(&self, peer_id: &str) -> bool {
        self.in_flight.lock().unwrap().contains(peer_id)
    }

    /// Whether the last exchange attempt with this peer failed within
    /// `FAILURE_COOLDOWN`.
    pub fn recently_failed(&self, peer_id: &str) -> bool {
        is_cooling_down(&self.cooldown.lock().unwrap(), peer_id, Instant::now())
    }

    /// See `ChannelService::forget_failures`; also used when a pair is
    /// removed.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn forget_failures(&self, peer_id: &str) {
        self.cooldown.lock().unwrap().remove(peer_id);
    }
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
