//! The Network channel over iroh (ADR-0009 D2, D9). Each device has one
//! Network endpoint: iroh's IP transport only, relays off (D10), bound to
//! one UDP port. One QUIC connection carries one session or one frame
//! before a session.
//!
//! iroh's own mDNS lookup (`iroh-mdns-address-lookup`) finds the other Fini
//! devices and gives their addresses. The key to connect to is the one
//! pinned for the pair, or for a pair request the one the other device
//! publishes; TLS proves it either way.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, TransportAddr};

use crate::models::channel::ChannelProblem;
use crate::models::pairing::DiscoveredDevice;
use crate::models::peer_frame::PeerFrame;
use crate::models::{Event, Identity};
use crate::repositories::{ChannelRepository, DeviceRepository};
use crate::services::channel_service::iroh_link::{IrohDataLink, PEER_ALPN, SYNC_ALPN};
use crate::services::channel_service::{
    send_frame, ChannelKind, ChannelService, DataLink, LinkHandlerSlot, LinkHandlers,
};
use crate::services::db::{open_db_at_path, utc_now};
use crate::services::event_bus_service::EventBusService;

/// How long a connect may take before it counts as failed. A peer on the
/// same network answers in milliseconds; one that went away never does.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the Network server waits before it tries its port again.
const REBIND_INTERVAL: Duration = Duration::from_secs(5);

/// After a failed attempt at a peer (rejected, port closed), wait this long
/// before the next, so that a peer that cannot take a session does not turn
/// every tick into another connection.
pub const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);

/// The mDNS service name for Fini's Network channel. Only Fini endpoints
/// use it, so other iroh apps on the network are not heard.
const MDNS_SERVICE_NAME: &str = "fini";

/// How long a device's last mDNS record keeps it on the network.
const HEARD_TIMEOUT: Duration = Duration::from_secs(150);

/// How long a device in Add Device stays in the Add Device list after its
/// last record.
const AVAILABLE_TIMEOUT: Duration = Duration::from_secs(15);

/// The UDP port of the Network endpoint, unless `FINI_SPACE_SYNC_WS_PORT`
/// gives another.
pub const NETWORK_PORT: u16 = 45_455;

/// One device the mDNS lookup heard.
#[derive(Debug, Clone)]
struct HeardDevice {
    hostname: String,
    addr: SocketAddr,
    /// The key the device announces. Unproven until a connection to it
    /// succeeds, so it is only used to connect, never to trust.
    endpoint_id: String,
    /// The device is in Add Device.
    add_device: bool,
    last_seen_at: String,
    last_seen: Instant,
}

pub struct NetworkChannelService {
    identity: Identity,
    db_path: PathBuf,
    bus: Arc<EventBusService>,
    /// The UDP port the endpoint binds; see `endpoint`.
    port: u16,
    /// Bound on first use.
    endpoint: tokio::sync::OnceCell<Endpoint>,
    /// Set once this channel serves. A serving endpoint must sit on `port`,
    /// the port the other devices connect to; only a channel that just
    /// connects out may fall back to any free port.
    serves: AtomicBool,
    /// Set once the mDNS lookup started; it starts once.
    discovery_started: AtomicBool,
    /// Set while the mDNS lookup does not run: the problem of this channel
    /// (ADR-0008 D19).
    discovery_stopped: AtomicBool,
    /// Whether Add Device is open on this device: published with the mDNS
    /// record.
    add_device_mode: AtomicBool,
    /// The devices the mDNS lookup heard, by device id.
    heard: Mutex<HashMap<String, HeardDevice>>,
    /// Peers with a connect in flight -- one at a time for each peer.
    in_flight: Mutex<HashSet<String>>,
    /// When this device's own connection to each peer became a session. For
    /// `CROSSING_WINDOW` after that, a connection from the peer is a
    /// crossing, not a restart (`connecting`).
    connected_at: Mutex<HashMap<String, Instant>>,
    /// Until when each peer whose last attempt failed is left alone.
    cooldown: Mutex<HashMap<String, Instant>>,
    handlers: LinkHandlerSlot,
    /// The test switch, for debug builds only. Off makes the network look
    /// absent to Fini; the real network stays on.
    #[cfg(feature = "devtools")]
    switched_off: AtomicBool,
}

impl NetworkChannelService {
    /// The channel on `NETWORK_PORT`, or on the port that
    /// `FINI_SPACE_SYNC_WS_PORT` gives.
    pub fn new(identity: Identity, db_path: PathBuf, bus: Arc<EventBusService>) -> Arc<Self> {
        let port = std::env::var("FINI_SPACE_SYNC_WS_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(NETWORK_PORT);
        Arc::new(Self::on_port(identity, db_path, bus, port))
    }

    pub fn on_port(identity: Identity, db_path: PathBuf, bus: Arc<EventBusService>, port: u16) -> Self {
        Self {
            identity,
            db_path,
            bus,
            port,
            endpoint: tokio::sync::OnceCell::new(),
            serves: AtomicBool::new(false),
            discovery_started: AtomicBool::new(false),
            discovery_stopped: AtomicBool::new(false),
            add_device_mode: AtomicBool::new(false),
            heard: Mutex::default(),
            in_flight: Mutex::default(),
            connected_at: Mutex::default(),
            cooldown: Mutex::default(),
            handlers: LinkHandlerSlot::default(),
            #[cfg(feature = "devtools")]
            switched_off: AtomicBool::new(false),
        }
    }

    /// Whether the test switch set the network to off. Always false in a
    /// build without `devtools`.
    pub fn switched_off(&self) -> bool {
        #[cfg(feature = "devtools")]
        return self.switched_off.load(Ordering::SeqCst);
        #[cfg(not(feature = "devtools"))]
        false
    }

    /// Sets the test switch; see `switched_off`. Off also forgets the
    /// devices heard, as a lost network does.
    #[cfg(feature = "devtools")]
    pub fn set_switch(&self, on: bool) {
        log::info!("[transport][network] test switch: network {}", if on { "on" } else { "off" });
        self.switched_off.store(!on, Ordering::SeqCst);
        if !on {
            self.heard.lock().unwrap().clear();
            self.bus.publish(Event::AvailableDevicesChanged);
        }
    }

    /// Marks this channel as one that serves. Call it before the first
    /// connect, or that connect could bind (and keep) a random port.
    fn mark_serving(&self) {
        self.serves.store(true, Ordering::SeqCst);
    }

    /// The UDP port the endpoint binds.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the mDNS lookup started.
    pub fn discovery_started(&self) -> bool {
        self.discovery_started.load(Ordering::SeqCst)
    }

    /// The endpoint, bound on first use to `port`. A second process for the
    /// same device (the CLI next to a running app) finds the port taken and
    /// binds any free one: it only connects out. A channel that serves
    /// fails instead and is tried again.
    pub async fn endpoint(&self) -> Result<Endpoint, String> {
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
            .secret_key(self.identity.secret_key.clone())
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![PEER_ALPN.to_vec(), SYNC_ALPN.to_vec()])
            .transport_config(crate::services::channel_service::iroh_link::transport_config())
            .clear_ip_transports()
            .bind_addr(SocketAddr::from(([0, 0, 0, 0], port)))
            .map_err(|err| err.to_string())?
            .bind()
            .await
            .map_err(|err| format!("binding the network endpoint failed: {err}"))
    }

    /// Connects to the device holding `peer_key` at `ip:port`, for `alpn`:
    /// `SYNC_ALPN` for a session, `PEER_ALPN` for the frames before one.
    pub async fn connect(&self, peer_key: &str, ip: IpAddr, port: u16, alpn: &[u8]) -> Result<Box<dyn DataLink>, String> {
        Ok(Box::new(self.connect_link(peer_key, ip, port, alpn).await?))
    }

    async fn connect_link(&self, peer_key: &str, ip: IpAddr, port: u16, alpn: &[u8]) -> Result<IrohDataLink, String> {
        if self.switched_off() {
            return Err("the test switch set the network to off".to_string());
        }
        let endpoint = self.endpoint().await?;
        let id: EndpointId = peer_key.parse().map_err(|err| format!("invalid peer key '{peer_key}': {err}"))?;
        let addr = EndpointAddr::from_parts(id, [TransportAddr::Ip(SocketAddr::new(ip, port))]);
        let connection = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(addr, alpn))
            .await
            .map_err(|_| format!("connect to {ip}:{port} timed out"))?
            .map_err(|err| format!("connect to {ip}:{port} failed: {err}"))?;
        IrohDataLink::open(ChannelKind::Network, connection, Some(ip.to_string())).await
    }

    /// Sends one frame before a session (a pair request, accept or
    /// completion) and lets the connection close once the peer has it.
    pub async fn send_one_frame(&self, peer_key: &str, ip: IpAddr, port: u16, frame: &PeerFrame) -> Result<(), String> {
        let mut link = self.connect_link(peer_key, ip, port, PEER_ALPN).await?;
        send_frame(&mut link, frame).await?;
        link.finish().await;
        Ok(())
    }

    /// Binds the endpoint and serves it: every connection a peer opens
    /// goes to the handler of its ALPN, through iroh's `Router`.
    async fn run_server(self: Arc<Self>) {
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
        self.serve(endpoint).await;
    }

    /// Binds the endpoint on a free port, serves it, and returns the port.
    #[cfg(test)]
    pub(crate) async fn spawn_server_on_free_port(self: Arc<Self>) -> u16 {
        let endpoint = self.bind(0).await.expect("bind the network endpoint");
        let port = bound_port(&endpoint);
        tokio::spawn(async move { self.serve(endpoint).await });
        port
    }

    async fn serve(self: Arc<Self>, endpoint: Endpoint) {
        let Some(handlers) = self.handlers.get().cloned() else {
            log::warn!("[transport][network] no link handlers; not serving");
            return;
        };
        let channel = self.clone();
        let switched_off = Arc::new(move || channel.switched_off());
        crate::services::channel_service::LinkAcceptor::serve(ChannelKind::Network, endpoint, handlers, switched_off)
            .await;
    }

    /// Finds other Fini devices on the network with iroh's own mDNS lookup,
    /// and announces this one. The test switch (debug builds) stops the
    /// lookup while the network is off, and starts a new one when it is on.
    async fn run_discovery(self: Arc<Self>) {
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
                    self.discovery_stopped.store(true, Ordering::SeqCst);
                    tokio::time::sleep(REBIND_INTERVAL).await;
                    continue;
                }
            };
            let Ok(lookup) = endpoint.address_lookup() else {
                log::warn!("[transport][network] the endpoint has no address lookup");
                self.discovery_stopped.store(true, Ordering::SeqCst);
                return;
            };
            lookup.add(mdns.clone());
            let mut published = self.user_data();
            endpoint.set_user_data_for_address_lookup(published.clone());
            self.discovery_stopped.store(false, Ordering::SeqCst);
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
                            if device_id == self.identity.device_id {
                                continue;
                            }
                            let Some(addr) = endpoint_info.data.ip_addrs().next().copied() else {
                                continue;
                            };
                            devices.insert(endpoint_id, device_id.clone());
                            self.record_heard(&device_id, &name, addr, &endpoint_id.to_string(), add_device);
                        }
                        Some(DiscoveryEvent::Expired { endpoint_id }) => {
                            if let Some(device_id) = devices.remove(&endpoint_id) {
                                self.remove_heard(&device_id);
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
                        let wanted = self.user_data();
                        if wanted != published {
                            endpoint.set_user_data_for_address_lookup(wanted.clone());
                            published = wanted;
                        }
                        self.keep_recent_records();
                    }
                }
            }
            lookup.clear();
            log::info!("[transport][network] mDNS lookup stopped");
        }
    }

    /// Records a device that the mDNS lookup heard. A device that arrives
    /// is the moment that waiting work can go to it.
    fn record_heard(&self, device_id: &str, hostname: &str, addr: SocketAddr, endpoint_id: &str, add_device: bool) {
        let (arrived, listed_before) = {
            let mut heard = self.heard.lock().unwrap();
            let previous = heard.get(device_id);
            let arrived = previous.is_none_or(|device| device.last_seen.elapsed() >= HEARD_TIMEOUT);
            let listed_before = previous.is_some_and(|device| device.add_device);
            heard.insert(
                device_id.to_string(),
                HeardDevice {
                    hostname: hostname.to_string(),
                    addr,
                    endpoint_id: endpoint_id.to_string(),
                    add_device,
                    last_seen_at: utc_now(),
                    last_seen: Instant::now(),
                },
            );
            (arrived, listed_before)
        };
        if arrived {
            log::info!("[transport][network] heard {device_id} at {addr}");
            self.bus.publish(Event::DeviceHeard { peer_device_id: device_id.to_string(), kind: ChannelKind::Network });
        }
        if arrived || listed_before != add_device {
            self.bus.publish(Event::AvailableDevicesChanged);
        }
    }

    /// The mDNS lookup does not hear the device any more.
    fn remove_heard(&self, device_id: &str) {
        if self.heard.lock().unwrap().remove(device_id).is_some() {
            self.bus.publish(Event::AvailableDevicesChanged);
        }
    }

    /// Keeps only the devices heard in the last `HEARD_TIMEOUT`.
    fn keep_recent_records(&self) {
        let removed = {
            let mut heard = self.heard.lock().unwrap();
            let before = heard.len();
            heard.retain(|_, device| device.last_seen.elapsed() < HEARD_TIMEOUT);
            before != heard.len()
        };
        if removed {
            self.bus.publish(Event::AvailableDevicesChanged);
        }
    }

    /// Every device the mDNS lookup heard recently, in Add Device or not.
    pub fn heard_devices(&self) -> Vec<DiscoveredDevice> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, device)| device.last_seen.elapsed() < HEARD_TIMEOUT)
            .map(|(device_id, device)| DiscoveredDevice {
                device_id: device_id.clone(),
                hostname: device.hostname.clone(),
                addr: device.addr.ip().to_string(),
                discovery_port: 0,
                ws_port: Some(device.addr.port()),
                last_seen_at: device.last_seen_at.clone(),
                channel_kind: ChannelKind::Network,
            })
            .collect()
    }

    /// Where the mDNS lookup last heard the device, and the key it
    /// announces, if it was heard recently.
    pub fn heard_address(&self, device_id: &str) -> Option<(SocketAddr, String)> {
        self.heard
            .lock()
            .unwrap()
            .get(device_id)
            .filter(|device| device.last_seen.elapsed() < HEARD_TIMEOUT)
            .map(|device| (device.addr, device.endpoint_id.clone()))
    }

    /// Starts a session with this peer unless one runs or is being made
    /// (ADR-0008 D10), the peer is not heard, or its last attempt failed
    /// within `FAILURE_COOLDOWN`.
    fn start_exchange(self: &Arc<Self>, peer_id: &str) {
        if self.handlers.has_session(peer_id, ChannelKind::Network) || self.heard_address(peer_id).is_none() {
            return;
        }
        if self.recently_failed(peer_id) {
            return;
        }
        if !self.in_flight.lock().unwrap().insert(peer_id.to_string()) {
            return;
        }
        let channel = self.clone();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            if !channel.exchange_with(&peer_id).await {
                channel.cooldown.lock().unwrap().insert(peer_id.clone(), Instant::now() + FAILURE_COOLDOWN);
                // Now, so that the work can go over Bluetooth meanwhile; and
                // once the cooldown is over, to try Network again.
                let changed = Event::ConnectChanged { peer_device_id: peer_id.clone(), kind: ChannelKind::Network };
                channel.bus.publish(changed.clone());
                let bus = channel.bus.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(FAILURE_COOLDOWN).await;
                    bus.publish(changed);
                });
            }
            channel.in_flight.lock().unwrap().remove(&peer_id);
        });
    }

    /// Whether the last attempt with this peer failed within
    /// `FAILURE_COOLDOWN`.
    pub fn recently_failed(&self, peer_id: &str) -> bool {
        is_cooling_down(&self.cooldown.lock().unwrap(), peer_id, Instant::now())
    }

    /// One session: connect to where the lookup last heard the peer and
    /// hold the session to its end. `false` if it could not start at all.
    async fn exchange_with(&self, peer_id: &str) -> bool {
        if !self.is_on(peer_id) {
            return true;
        }
        let Some((addr, _)) = self.heard_address(peer_id) else {
            return true;
        };
        let Some(peer_key) = self.pinned_key(peer_id) else {
            // ADR-0009 D8: a pair from before keys existed is paired again.
            log::warn!("[transport][network] {peer_id} has no key pinned; pair the devices again");
            return false;
        };
        let Some(handlers) = self.handlers.get().cloned() else {
            return false;
        };
        log::info!("[transport][network] connecting to {peer_id} at {addr}");
        // TLS proves the peer holds the pinned key; the peer may still
        // reject, by closing the connection with a code.
        let link = match self.connect(&peer_key, addr.ip(), addr.port(), SYNC_ALPN).await {
            Ok(link) => link,
            Err(err) => {
                log::warn!("[transport][network] connect to {peer_id} failed: {err}");
                return false;
            }
        };
        let held = handlers
            .session
            .hold(
                link,
                peer_id,
                ChannelKind::Network,
                Box::new(|| {
                    // Held, not connecting; for `CROSSING_WINDOW` a
                    // connection from the peer is still a crossing.
                    self.connected_at.lock().unwrap().insert(peer_id.to_string(), Instant::now());
                    self.in_flight.lock().unwrap().remove(peer_id);
                    log::info!("[transport][network] holding a session with {peer_id}");
                }),
            )
            .await;
        // Rejected by the peer: a failed attempt, so the next one waits
        // `FAILURE_COOLDOWN`. A session that runs here already is none.
        !matches!(held, Ok(close_code) if crate::services::space_sync_service::rejected_by_peer(close_code))
    }

    fn pinned_key(&self, peer_id: &str) -> Option<String> {
        tokio::task::block_in_place(|| DeviceRepository::new(&mut open_db_at_path(&self.db_path)).pinned_key(peer_id))
    }

    /// The user data this device publishes with its iroh mDNS record: a
    /// version, the Add Device flag, the device id and the name, as
    /// `fini1;a=<0|1>;d=<device id>;n=<name>`. iroh limits it to 245 bytes,
    /// so the name is cut to fit.
    fn user_data(&self) -> Option<iroh::endpoint_info::UserData> {
        let add_device = self.add_device_mode.load(Ordering::SeqCst);
        let mut name = self.identity.hostname.clone();
        let prefix = format!("fini1;a={};d={};n=", u8::from(add_device), self.identity.device_id);
        while prefix.len() + name.len() > 245 {
            name.pop();
        }
        format!("{prefix}{name}").parse().ok()
    }
}

#[async_trait::async_trait]
impl ChannelService for NetworkChannelService {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Network
    }

    fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn set_link_handlers(&self, handlers: LinkHandlers) {
        self.handlers.set(handlers);
    }

    /// Every platform Fini runs on has a network stack.
    fn available(&self) -> bool {
        true
    }

    /// Starts the mDNS lookup once. `FINI_DISCOVERY_DISABLED=1` keeps it
    /// off.
    fn start_discovery(self: Arc<Self>) {
        if self.discovery_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if std::env::var("FINI_DISCOVERY_DISABLED").ok().as_deref() != Some("1") {
            tauri::async_runtime::spawn(self.run_discovery());
        }
    }

    fn start_serving(self: Arc<Self>) {
        // Before the spawn: a connect that runs first must already see that
        // this channel serves, or it could bind (and keep) a random port.
        self.mark_serving();
        tauri::async_runtime::spawn(self.run_server());
    }

    /// Holds a session with every paired device that has the Network
    /// channel on and is heard.
    fn keep_serving(self: Arc<Self>) {
        let peers = tokio::task::block_in_place(|| {
            ChannelRepository::new(&mut open_db_at_path(&self.db_path)).peers_with_channel_enabled(ChannelKind::Network)
        });
        for peer in peers {
            self.start_exchange(&peer);
        }
    }

    /// A connection to where the mDNS lookup last heard the peer. The
    /// lookup finds the peer, so `within` is not needed: a peer not heard
    /// is an error at once.
    async fn connect_plain(&self, peer_device_id: &str, _within: Duration) -> Result<Box<dyn DataLink>, String> {
        let (addr, _) = self
            .heard_address(peer_device_id)
            .ok_or_else(|| format!("{peer_device_id} is not heard on the network"))?;
        let peer_key = self
            .pinned_key(peer_device_id)
            // ADR-0009 D8: a pair from before keys existed is paired again.
            .ok_or_else(|| format!("{peer_device_id} has no key pinned; pair the devices again"))?;
        self.connect(&peer_key, addr.ip(), addr.port(), PEER_ALPN).await
    }

    /// This device's own connection is being made, or became a session in
    /// the last `CROSSING_WINDOW`.
    fn connecting(&self, peer_device_id: &str) -> bool {
        self.in_flight.lock().unwrap().contains(peer_device_id)
            || self
                .connected_at
                .lock()
                .unwrap()
                .get(peer_device_id)
                .is_some_and(|at| at.elapsed() < crate::services::space_sync_service::CROSSING_WINDOW)
    }

    fn forget_failures(&self, peer_device_id: &str) {
        self.cooldown.lock().unwrap().remove(peer_device_id);
    }

    fn problem(&self) -> Option<ChannelProblem> {
        self.discovery_stopped.load(Ordering::SeqCst).then_some(ChannelProblem::NetworkUnavailable)
    }

    fn set_add_device_mode(&self, on: bool) {
        self.add_device_mode.store(on, Ordering::SeqCst);
        self.bus.publish(Event::AvailableDevicesChanged);
    }

    /// The devices heard with the Add Device flag in the last
    /// `AVAILABLE_TIMEOUT`, while Add Device is open here.
    fn available_devices(&self) -> Vec<DiscoveredDevice> {
        if !self.add_device_mode.load(Ordering::SeqCst) {
            return Vec::new();
        }
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, device)| device.add_device && device.last_seen.elapsed() <= AVAILABLE_TIMEOUT)
            .map(|(device_id, device)| DiscoveredDevice {
                device_id: device_id.clone(),
                hostname: device.hostname.clone(),
                addr: device.addr.ip().to_string(),
                discovery_port: 0,
                ws_port: Some(device.addr.port()),
                last_seen_at: device.last_seen_at.clone(),
                channel_kind: ChannelKind::Network,
            })
            .collect()
    }
}

/// The UDP port the Network endpoint is bound to.
#[cfg(test)]
pub(crate) fn bound_port(endpoint: &Endpoint) -> u16 {
    endpoint.bound_sockets().first().map(|addr| addr.port()).expect("the network endpoint has a socket")
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

    #[test]
    fn user_data_round_trips() {
        assert_eq!(
            parse_user_data("fini1;a=1;d=dev-1;n=laptop"),
            Some((true, "dev-1".to_string(), "laptop".to_string()))
        );
        assert_eq!(parse_user_data("other"), None);
    }
}
