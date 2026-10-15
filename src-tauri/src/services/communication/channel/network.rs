//! The Network channel over iroh (ADR-0009 D2, D9). Each device has one
//! Network endpoint: iroh's IP transport only, relays off (D10), bound to
//! `DeviceConnectionState::space_sync_ws_port` over UDP. One QUIC connection
//! carries one exchange or one pairing frame (`iroh_link::IrohDataLink`).
//!
//! iroh's own mDNS lookup (`iroh-mdns-address-lookup`) finds the other Fini
//! devices and gives their addresses. The key to dial is the one pinned for
//! the pair, or for a pair request the one the other device publishes; TLS
//! proves it either way.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
#[cfg(any(feature = "ui-plane", test))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use iroh::endpoint::presets;
#[cfg(any(feature = "ui-plane", test))]
use iroh::endpoint::IncomingAddr;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr};

use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::DeviceConnectionState;
use crate::services::communication::sync::session;
use crate::services::communication::channel::iroh_link::{IrohDataLink, PEER_ALPN, SYNC_ALPN};
use crate::services::communication::channel::service::ChannelService;
use crate::services::communication::channel::{send_frame, ChannelKind, DataLink};
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::pairing::ChannelProblem;
use crate::services::communication::sync::types::PeerFrame;

/// How long a dial may take before it counts as failed. A peer on the same
/// network answers in milliseconds; one that went away never does.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the Network server waits before trying its port again.
const REBIND_INTERVAL: Duration = Duration::from_secs(5);

/// After a failed attempt at a present peer (refused, unreachable port),
/// wait this long before the next, so a peer that cannot take an exchange
/// does not turn every wake into another connection.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);

/// The mDNS service name for Fini's Network channel. Only Fini endpoints use
/// it, so other iroh apps on the network are not heard.
const MDNS_SERVICE_NAME: &str = "fini";

/// The Network channel of one `DeviceConnectionState`: its iroh endpoint,
/// its mDNS lookup, and its exchange attempts. Per state, not per process:
/// tests run two devices in one process.
pub struct NetworkChannel {
    secret_key: SecretKey,
    /// The UDP port the endpoint binds; see `endpoint`.
    port: u16,
    /// Bound on first use.
    endpoint: tokio::sync::OnceCell<Endpoint>,
    /// Set once this channel serves. A serving endpoint must sit on `port`,
    /// the port the other devices connect to; only a channel that just dials
    /// out may fall back to any free port.
    serves: AtomicBool,
    /// Set once the mDNS lookup started; it starts once per channel.
    discovery_started: AtomicBool,
    /// Peers with an exchange attempt in flight -- one at a time per peer.
    in_flight: Mutex<HashSet<String>>,
    /// When this device's own connection to each peer became a session. For
    /// `session::CROSSING_WINDOW` after that, a connection from the peer is a crossing,
    /// not a restart (`connecting`).
    connected_at: Mutex<HashMap<String, Instant>>,
    /// Until when each peer whose last attempt failed is left alone.
    cooldown: Mutex<HashMap<String, Instant>>,
    /// The test switch, for debug builds only. Off makes the network look
    /// absent to Fini: no announcements go out, none come in, and
    /// connections fail. The real network stays on, so that no test must
    /// switch off the Wi-Fi of a person's device.
    #[cfg(feature = "devtools")]
    switched_off: AtomicBool,
}

impl NetworkChannel {
    pub(crate) fn new(secret_key: SecretKey, port: u16) -> Self {
        Self {
            secret_key,
            port,
            endpoint: tokio::sync::OnceCell::new(),
            serves: AtomicBool::new(false),
            discovery_started: AtomicBool::new(false),
            in_flight: Mutex::default(),
            connected_at: Mutex::default(),
            cooldown: Mutex::default(),
            #[cfg(feature = "devtools")]
            switched_off: AtomicBool::new(false),
        }
    }

    /// Whether the test switch set the network to off. Always false in a
    /// build without `devtools`.
    pub(crate) fn switched_off(&self) -> bool {
        #[cfg(feature = "devtools")]
        return self.switched_off.load(Ordering::SeqCst);
        #[cfg(not(feature = "devtools"))]
        false
    }

    /// Sets the test switch; see `switched_off`.
    #[cfg(feature = "devtools")]
    pub fn set_switch(&self, on: bool) {
        log::info!("[transport][network] test switch: network {}", if on { "on" } else { "off" });
        self.switched_off.store(!on, Ordering::SeqCst);
    }

    /// Marks this channel as one that serves. Call it before the first dial,
    /// or that dial could bind (and keep) a random port.
    pub(crate) fn mark_serving(&self) {
        self.serves.store(true, Ordering::SeqCst);
    }

    /// Whether the mDNS lookup started.
    pub(crate) fn discovery_started(&self) -> bool {
        self.discovery_started.load(Ordering::SeqCst)
    }

    /// The endpoint, bound on first use to `port`. A second process for the
    /// same device (the CLI next to a running app) finds the port taken and
    /// binds any free one: it only dials out. A channel that serves never
    /// does: the other devices connect to `port`, so it fails instead and is
    /// tried again.
    pub(crate) async fn endpoint(&self) -> Result<Endpoint, String> {
        self.bind(self.port).await
    }

    async fn bind(&self, port: u16) -> Result<Endpoint, String> {
        let serves = self.serves.load(Ordering::SeqCst);
        self.endpoint
            .get_or_try_init(|| async {
                match self.bind_on(port).await {
                    Ok(endpoint) => Ok(endpoint),
                    Err(err) if port != 0 && !serves => {
                        log::warn!("[transport][network] port {port} unavailable ({err}); binding any free port");
                        self.bind_on(0).await
                    }
                    Err(err) => Err(err),
                }
            })
            .await
            .cloned()
    }

    async fn bind_on(&self, port: u16) -> Result<Endpoint, String> {
        Endpoint::builder(presets::Minimal)
            .secret_key(self.secret_key.clone())
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![PEER_ALPN.to_vec(), SYNC_ALPN.to_vec()])
            .transport_config(crate::services::communication::channel::iroh_link::transport_config())
            .clear_ip_transports()
            .bind_addr(SocketAddr::from(([0, 0, 0, 0], port)))
            .map_err(|err| err.to_string())?
            .bind()
            .await
            .map_err(|err| format!("binding the network endpoint failed: {err}"))
    }

    /// Connects to the device holding `peer_key` at `ip:port`, for `alpn`:
    /// `SYNC_ALPN` for a session, `PEER_ALPN` for the frames before one.
    pub async fn connect(
        &self, peer_key: &str, ip: IpAddr, port: u16, alpn: &[u8],
    ) -> Result<Box<dyn DataLink>, String> {
        Ok(Box::new(self.connect_link(peer_key, ip, port, alpn).await?))
    }

    async fn connect_link(&self, peer_key: &str, ip: IpAddr, port: u16, alpn: &[u8]) -> Result<IrohDataLink, String> {
        if self.switched_off() {
            return Err("the test switch set the network to off".to_string());
        }
        let endpoint = self.endpoint().await?;
        let id: EndpointId = peer_key
            .parse()
            .map_err(|err| format!("invalid peer key '{peer_key}': {err}"))?;
        let addr = EndpointAddr::from_parts(id, [TransportAddr::Ip(SocketAddr::new(ip, port))]);
        let connection = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(addr, alpn))
            .await
            .map_err(|_| format!("connect to {ip}:{port} timed out"))?
            .map_err(|err| format!("connect to {ip}:{port} failed: {err}"))?;
        IrohDataLink::open(ChannelKind::Network, connection, Some(ip.to_string())).await
    }

    /// Sends one pre-auth frame (a pair request, accept or completion) and
    /// lets the connection close once the peer has it.
    pub async fn send_one_frame(&self, peer_key: &str, ip: IpAddr, port: u16, frame: &PeerFrame) -> Result<(), String> {
        let mut link = self.connect_link(peer_key, ip, port, PEER_ALPN).await?;
        send_frame(&mut link, frame).await?;
        link.finish().await;
        Ok(())
    }

    /// Runs the accept loop on the endpoint and hands every connection to
    /// the shared gate (`crate::services::communication::pairing::run_peer_gate`).
    /// `ui-plane`/`test` only -- see `run_peer_gate`'s doc comment.
    #[cfg(any(feature = "ui-plane", test))]
    pub async fn run_server(&self, state: DeviceConnectionState, db_path: PathBuf) {
        self.mark_serving();
        let endpoint = loop {
            match self.endpoint().await {
                Ok(endpoint) => break endpoint,
                Err(err) => {
                    log::warn!("[transport][network] {err}; trying again in {REBIND_INTERVAL:?}");
                    tokio::time::sleep(REBIND_INTERVAL).await;
                }
            }
        };
        log::info!("[transport][network] listening on {:?}", endpoint.bound_sockets());
        self.serve(state, db_path, endpoint).await;
    }

    /// Serves the endpoint bound on `port`.
    #[cfg(test)]
    pub(crate) async fn run_server_on_port(&self, state: DeviceConnectionState, db_path: PathBuf, port: u16) {
        let endpoint = self.bind(port).await.expect("bind the network endpoint");
        self.serve(state, db_path, endpoint).await;
    }

    /// Binds the endpoint on a free port, serves it, and returns the port.
    #[cfg(test)]
    pub(crate) async fn spawn_server_on_free_port(&self, state: DeviceConnectionState, db_path: PathBuf) -> u16 {
        let endpoint = self.bind(0).await.expect("bind the network endpoint");
        let port = bound_port(&endpoint);
        tokio::spawn(async move { state.network.clone().serve(state, db_path, endpoint).await });
        port
    }

    #[cfg(any(feature = "ui-plane", test))]
    async fn serve(&self, state: DeviceConnectionState, db_path: PathBuf, endpoint: Endpoint) {
        while let Some(incoming) = endpoint.accept().await {
            if self.switched_off() {
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
                let is_session = connection.alpn() == SYNC_ALPN;
                match IrohDataLink::accept(ChannelKind::Network, connection, peer_addr).await {
                    Ok(link) if is_session => {
                        crate::services::communication::pairing::run_session_gate(Box::new(link), state, db_path).await;
                    }
                    Ok(link) => {
                        crate::services::communication::pairing::run_peer_gate(Box::new(link), state, db_path).await;
                    }
                    Err(err) => log::warn!("[transport][network] {err}"),
                }
            });
        }
    }

    /// Starts the mDNS lookup (`run_discovery`) once per channel. Asking
    /// again does nothing. `FINI_DISCOVERY_DISABLED=1` keeps it off.
    pub(crate) fn start_discovery(&self, state: &DeviceConnectionState) {
        if self.discovery_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if std::env::var("FINI_DISCOVERY_DISABLED").ok().as_deref() != Some("1") {
            let state = state.clone();
            tauri::async_runtime::spawn(async move { state.network.clone().run_discovery(&state).await });
        }
    }

    /// Finds other Fini devices on the network with iroh's own mDNS lookup
    /// (`iroh-mdns-address-lookup`), and announces this one. What it hears
    /// goes to the same records the rest of the Network channel reads:
    /// presence for paired devices, and the Add Device list.
    ///
    /// The test switch (debug builds) stops the lookup while the network is
    /// off and starts a new one when it is on, which asks the network again
    /// at once.
    async fn run_discovery(&self, state: &DeviceConnectionState) {
        use futures_util::StreamExt;
        use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};

        let endpoint = loop {
            match self.endpoint().await {
                Ok(endpoint) => break endpoint,
                Err(err) => {
                    log::warn!("[transport][network] discovery waits for the endpoint: {err}");
                    tokio::time::sleep(REBIND_INTERVAL).await;
                }
            }
        };
        loop {
            while self.switched_off() {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let mdns = match MdnsAddressLookup::builder().service_name(MDNS_SERVICE_NAME).build(endpoint.id()) {
                Ok(mdns) => mdns,
                Err(err) => {
                    log::warn!("[transport][network] mDNS lookup failed to start: {err}");
                    state.set_network_discovery_stopped(true);
                    tokio::time::sleep(REBIND_INTERVAL).await;
                    continue;
                }
            };
            let Ok(lookup) = endpoint.address_lookup() else {
                log::warn!("[transport][network] the endpoint has no address lookup");
                state.set_network_discovery_stopped(true);
                return;
            };
            lookup.add(mdns.clone());
            let mut published = user_data(state);
            endpoint.set_user_data_for_address_lookup(published.clone());
            state.set_network_discovery_stopped(false);
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
                                .and_then(|data| parse_user_data(data.as_ref()))
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
                            state.record_network_device(&device_id, &name, addr, &endpoint_id.to_string(), add_device);
                        }
                        Some(DiscoveryEvent::Expired { endpoint_id }) => {
                            if let Some(device_id) = devices.remove(&endpoint_id) {
                                state.remove_network_device(&device_id);
                            }
                        }
                        Some(_) => {}
                        None => break,
                    },
                    _ = tick.tick() => {
                        if self.switched_off() {
                            break;
                        }
                        // Add Device opened or closed: publish the new flag.
                        let wanted = user_data(state);
                        if wanted != published {
                            endpoint.set_user_data_for_address_lookup(wanted.clone());
                            published = wanted;
                        }
                        state.keep_recent_network_records();
                    }
                }
            }
            lookup.clear();
            log::info!("[transport][network] mDNS lookup stopped");
        }
    }

    /// Starts an exchange with this peer over the Network channel unless one
    /// is running or being attempted (ADR-0008 D10). Called when there is
    /// work for the peer; does nothing while the peer is not present on the
    /// network -- the lookup hearing it wakes the keeper, which asks again.
    pub fn start_exchange(&self, state: &DeviceConnectionState, peer_id: &str) {
        if state.has_session_on(peer_id, ChannelKind::Network) || !state.network_peer_available(peer_id) {
            return;
        }
        if self.recently_failed(peer_id) {
            return;
        }
        if !self.in_flight.lock().unwrap().insert(peer_id.to_string()) {
            return;
        }
        let state = state.clone();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            let channel = state.network.clone();
            if !channel.exchange_with(&state, &peer_id).await {
                channel
                    .cooldown
                    .lock()
                    .unwrap()
                    .insert(peer_id.clone(), Instant::now() + FAILURE_COOLDOWN);
                // Look again now, while the failure still counts, so the work
                // can go over Bluetooth meanwhile; and once the cooldown is
                // over, to try Network again.
                state.space_sync.wake();
                state.space_sync.wake_after(FAILURE_COOLDOWN);
            }
            channel.in_flight.lock().unwrap().remove(&peer_id);
        });
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

    /// One exchange: connect to where the lookup last heard the peer,
    /// authenticate, and run the exchange to its idle end. `false` if it
    /// could not be started at all.
    async fn exchange_with(&self, state: &DeviceConnectionState, peer_id: &str) -> bool {
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
        log::info!("[transport][network] connecting to {peer_id} at {target_addr}:{ws_port}");
        // TLS proves the peer holds the pinned key; the peer's gate may still
        // refuse, by closing the connection with a code (`run_session_gate`).
        let link = match self.connect(&peer_key, target_addr, ws_port, SYNC_ALPN).await {
            Ok(link) => link,
            Err(err) => {
                log::warn!("[transport][network] connect to {peer_id} failed: {err}");
                return false;
            }
        };
        let held = state
            .space_sync
            .hold_session(state, link, peer_id, ChannelKind::Network, || {
                // Held, not connecting; for `CROSSING_WINDOW` a connection
                // from the peer is still a crossing (`connecting`).
                self.connected_at.lock().unwrap().insert(peer_id.to_string(), Instant::now());
                self.in_flight.lock().unwrap().remove(peer_id);
                log::info!("[transport][network] holding a session with {peer_id}");
            })
            .await;
        // Refused by the peer's gate: a failed attempt, so the next one
        // waits `FAILURE_COOLDOWN` instead of connecting again at once. A
        // session already running here (its own connection won) is none.
        !matches!(held, Ok(close_code) if session::refused_by_peer(close_code))
    }
}

#[async_trait::async_trait]
impl ChannelService for NetworkChannel {
    /// Every platform Fini runs on has a network stack. Whether a *peer* is
    /// reachable over it is `is_green`'s question, not this one.
    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool {
        true
    }

    fn start_discovery(&self, state: &DeviceConnectionState) {
        NetworkChannel::start_discovery(self, state);
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self, state: &DeviceConnectionState) {
        // Before the spawn: a dial that runs first must already see that
        // this channel serves, or it could bind (and keep) a random port.
        self.mark_serving();
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            let db_path = state.db_path.clone();
            state.network.clone().run_server(state, db_path).await
        });
    }

    /// Green means a live connection: a held session with the peer
    /// (`session::idle_after`), not only an mDNS record.
    fn is_green(&self, state: &DeviceConnectionState, peer_device_id: &str) -> bool {
        state.has_session_on(peer_device_id, ChannelKind::Network)
    }

    /// Holds a session with every paired device that has the Network
    /// channel on: on each tick, connects to those the mDNS lookup hears and
    /// that have no session. `start_exchange` skips a device that has one,
    /// is not heard, or failed within `FAILURE_COOLDOWN`.
    fn keep_serving(&self, state: &DeviceConnectionState) {
        let peers = tokio::task::block_in_place(|| {
            let mut conn = open_db_at_path(&state.db_path);
            crate::services::communication::pairing::channels::peers_with_channel_enabled(
                &mut conn,
                ChannelKind::Network,
            )
        });
        for peer in peers {
            self.start_exchange(state, &peer);
        }
    }

    /// A connection to wherever the mDNS lookup last heard the peer. The
    /// lookup, not this, finds the peer, so `within` is not needed: a peer
    /// not heard is an error at once.
    #[cfg(any(feature = "ui-plane", test))]
    async fn connect_plain(
        &self, state: &DeviceConnectionState, peer_device_id: &str, _within: Duration,
    ) -> Result<Box<dyn DataLink>, String> {
        let (addr, port) = state
            .network_presence_address(peer_device_id)
            .ok_or_else(|| format!("{peer_device_id} is not heard on the network"))?;
        let ip: IpAddr = addr.parse().map_err(|err| format!("invalid peer addr '{addr}': {err}"))?;
        let peer_key = tokio::task::block_in_place(|| {
            crate::services::communication::pairing::pinned_key(&mut open_db_at_path(&state.db_path), peer_device_id)
        })
        // ADR-0009 D8: a pair from before keys existed is paired again.
        .ok_or_else(|| format!("{peer_device_id} has no key pinned; pair the devices again"))?;
        self.connect(&peer_key, ip, port, PEER_ALPN).await
    }

    #[cfg(any(feature = "ui-plane", test))]
    /// This device's own connection is being made, or became a session
    /// within `CROSSING_WINDOW`.
    fn connecting(&self, peer_device_id: &str) -> bool {
        self.in_flight.lock().unwrap().contains(peer_device_id)
            || self
                .connected_at
                .lock()
                .unwrap()
                .get(peer_device_id)
                .is_some_and(|at| at.elapsed() < session::CROSSING_WINDOW)
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn forget_failures(&self, peer_device_id: &str) {
        NetworkChannel::forget_failures(self, peer_device_id);
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn problem(&self, state: &DeviceConnectionState) -> Option<ChannelProblem> {
        state.network_discovery_stopped().then_some(ChannelProblem::NetworkUnavailable)
    }
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

/// The user data this device publishes with its iroh mDNS record: a version,
/// the Add Device flag, the device id and the name, as
/// `fini1;a=<0|1>;d=<device id>;n=<name>`. iroh limits it to 245 bytes, so the
/// name is cut to fit.
fn user_data(state: &DeviceConnectionState) -> Option<iroh::endpoint_info::UserData> {
    let add_device = state.is_add_mode_enabled();
    let mut name = state.identity.hostname.clone();
    let prefix = format!("fini1;a={};d={};n=", u8::from(add_device), state.identity.device_id);
    while prefix.len() + name.len() > 245 {
        name.pop();
    }
    format!("{prefix}{name}").parse().ok()
}

/// What another device's user data says: Add Device flag, device id, name.
fn parse_user_data(data: &str) -> Option<(bool, String, String)> {
    let rest = data.strip_prefix("fini1;a=")?;
    let (flag, rest) = rest.split_once(";d=")?;
    let (device_id, name) = rest.split_once(";n=")?;
    Some((flag == "1", device_id.to_string(), name.to_string()))
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
