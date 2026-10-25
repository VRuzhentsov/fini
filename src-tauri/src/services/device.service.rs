//! The device service (ADR-0010 D4): the paired devices and how a pair is
//! made, changed and removed -- Add Device and pairing, the setup of a
//! channel, the channel switches, unlink.
//!
//! It takes the links of the frames before a session (`PeerLinkHandler`,
//! ALPN `fini/peer/1`, and every plain Bluetooth link). It uses the channel
//! services to reach a device, and tells SpaceSync about changes through
//! the event bus, never by a call (D15).

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use diesel::sqlite::SqliteConnection;
use diesel::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{Device, Event, Identity};
use crate::repositories::{ChannelRepository, DeviceRepository, OutboxRepository};
use crate::services::bluetooth_channel_service::BluetoothChannelService;
use crate::services::channel_service::{
    recv_frame, send_frame, ChannelKind, ChannelProblem, ChannelService, DataLink, DiscoveredDevice,
    PairAcceptPayload, PairCompletePayload, PairRequestPayload, PeerFrame, PeerLinkHandler,
};
use crate::services::db::{open_db_at_path, utc_now};
use crate::services::event_bus_service::EventBusService;
use crate::services::network_channel_service::NetworkChannelService;

/// The protocol name in every pairing frame.
pub const DISCOVERY_PROTOCOL: &str = "fini-device-sync-v1";
/// The UDP port of the old discovery beacon, still sent in a pair request.
pub const DISCOVERY_PORT: u16 = 45_454;
/// How long a pair request waits for an answer.
pub const PAIR_REQUEST_TTL_SECS: i64 = 60;

/// How long one search for the peer may take before the `AddChannel` is
/// tried again (ADR-0008 D12).
#[cfg(not(test))]
const SETUP_SEARCH: Duration = Duration::from_secs(60);
#[cfg(test)]
const SETUP_SEARCH: Duration = Duration::from_millis(200);

/// The wait after an `AddChannel` that found no peer or got no accept, so
/// that it is not sent again in a tight loop.
const SETUP_RETRY: Duration = Duration::from_secs(2);

/// How long the peer may take to accept an `AddChannel` once it has it.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the identify step waits for the answer once the link is open.
#[cfg(any(target_os = "linux", target_os = "android"))]
const IDENTIFY_ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// A pair request that arrived, with what answering it needs.
#[derive(Debug, Clone)]
struct StoredPairRequest {
    request: IncomingPairRequest,
    from_addr: String,
    from_ws_port: Option<u16>,
    /// The requester's key, as TLS proved it on the request's connection.
    from_endpoint_id: Option<String>,
}

impl StoredPairRequest {
    /// A pair request as this device stores it when it arrives. Its expiry
    /// is counted from this device's clock, not the sender's.
    fn from_payload(payload: &PairRequestPayload, from_addr: String, from_endpoint_id: Option<String>, via_bluetooth: bool) -> Self {
        let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        Self {
            request: IncomingPairRequest {
                request_id: payload.request_id.clone(),
                from_device_id: payload.from_device_id.clone(),
                from_hostname: payload.from_hostname.clone(),
                created_at: utc_now(),
                expires_at,
                attempts: 0,
                cooldown_until: None,
                via_bluetooth,
                from_bluetooth_address: via_bluetooth.then(|| from_addr.clone()),
            },
            from_addr,
            from_ws_port: payload.from_ws_port,
            from_endpoint_id,
        }
    }

    fn expired(&self) -> bool {
        chrono::DateTime::parse_from_rfc3339(&self.request.expires_at)
            .map(|expires_at| Utc::now() >= expires_at.with_timezone(&Utc))
            .unwrap_or(true)
    }
}

/// What pairing and Add Device keep in memory.
#[derive(Default)]
struct PairingState {
    add_mode_enabled: bool,
    tx_count: u64,
    rx_count: u64,
    incoming_requests: HashMap<String, StoredPairRequest>,
    outgoing_code_updates: HashMap<String, PairCodeUpdate>,
    outgoing_pair_completions: HashMap<String, PairCompletionUpdate>,
    /// The peer's key for each pairing this device completed or saw
    /// completed, by request id: `(peer device id, key)` (ADR-0009 D8).
    pairing_keys: HashMap<String, (String, String)>,
    /// The channel setups open on this device, and how far each init got
    /// (ADR-0008 D1, D2).
    channel_setups: HashMap<(String, ChannelKind), ChannelSetup>,
}

impl PairingState {
    fn prune_expired_requests(&mut self) {
        self.incoming_requests.retain(|_, request| !request.expired());
    }
}

pub struct DeviceService {
    identity: Identity,
    db_path: PathBuf,
    bus: Arc<EventBusService>,
    network: Arc<NetworkChannelService>,
    bluetooth: Arc<BluetoothChannelService>,
    me: Weak<DeviceService>,
    state: Mutex<PairingState>,
    /// The links that Bluetooth pair requests arrived on, by request id: the
    /// accept and the complete answer on them.
    parked_links: Mutex<HashMap<String, Box<dyn DataLink>>>,
}

impl DeviceService {
    pub fn new(
        identity: Identity,
        db_path: PathBuf,
        bus: Arc<EventBusService>,
        network: Arc<NetworkChannelService>,
        bluetooth: Arc<BluetoothChannelService>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            identity,
            db_path,
            bus,
            network,
            bluetooth,
            me: me.clone(),
            state: Mutex::default(),
            parked_links: Mutex::default(),
        })
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("the device service lives as long as its tasks")
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut SqliteConnection) -> T) -> T {
        tokio::task::block_in_place(|| f(&mut open_db_at_path(&self.db_path)))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, PairingState>, String> {
        self.state.lock().map_err(|_| "the pairing state lock is poisoned".to_string())
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    fn channel(&self, kind: ChannelKind) -> Arc<dyn ChannelService> {
        match kind {
            ChannelKind::Network => self.network.clone(),
            ChannelKind::Bluetooth => self.bluetooth.clone(),
        }
    }

    /// Removes the row of a channel that the peer has no row for, unless a
    /// setup of it is open here: then the setup is not done there yet.
    pub fn start(&self) {
        let service = self.arc();
        tauri::async_runtime::spawn(async move {
            let mut events = service.bus.subscribe();
            loop {
                match events.recv().await {
                    Ok(Event::ChannelMissingAtPeer { peer_device_id, kind }) => {
                        service.remove_channel_missing_at_peer(&peer_device_id, kind)
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    fn remove_channel_missing_at_peer(&self, peer_device_id: &str, kind: ChannelKind) {
        if self.has_channel_setup(peer_device_id, kind) {
            return;
        }
        match self.with_conn(|conn| ChannelRepository::new(conn).remove(peer_device_id, kind)) {
            Ok(()) => {
                log::info!("[session] {peer_device_id}: peer has no {kind:?} channel; removed it here");
                self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
            }
            Err(err) => log::warn!("[session] {peer_device_id}: removing {kind:?} failed: {err}"),
        }
    }

    // ---- Add Device ----

    /// Opens or closes Add Device on this device. `bluetooth` is false when
    /// only the Network channel is being added: then the Bluetooth half
    /// (the add flag in the advertisement, the Android permission prompt)
    /// stays off.
    pub fn set_add_device_mode(&self, on: bool, bluetooth: bool) -> Result<(), String> {
        {
            let mut state = self.lock()?;
            state.add_mode_enabled = on;
            if !on {
                state.incoming_requests.clear();
                state.outgoing_code_updates.clear();
                state.outgoing_pair_completions.clear();
                state.pairing_keys.clear();
            }
        }
        log::info!(
            "[device-sync] add mode {} for {} ({})",
            if on { "enabled" } else { "disabled" },
            self.identity.hostname,
            self.identity.device_id
        );
        self.network.set_add_device_mode(on);
        if bluetooth || !on {
            self.bluetooth.set_add_device_mode(on);
        }
        // Opening Add Device is a person's act: the right moment to ask for
        // the Nearby Devices permission, once for each opening.
        #[cfg(target_os = "android")]
        if on && bluetooth {
            crate::services::ble_plugin::request_permission();
        }
        self.bus.publish(Event::AvailableDevicesChanged);
        self.bus.publish(Event::PairRequestsChanged);
        Ok(())
    }

    /// Every device the Network channel heard recently.
    pub fn heard_devices(&self) -> Vec<DiscoveredDevice> {
        self.network.heard_devices()
    }

    pub fn is_add_mode_enabled(&self) -> bool {
        self.lock().map(|state| state.add_mode_enabled).unwrap_or(false)
    }

    /// The Add Device list: the devices in Add Device that the channels
    /// heard. A Bluetooth device that an identify step did not answer yet
    /// has the id `bt:<fingerprint>` and its short advertised name; a
    /// device paired here is listed with its id and name, so that the list
    /// can offer "Pair again".
    pub fn available_devices(&self) -> Vec<DiscoveredDevice> {
        let mut devices = self.network.available_devices();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        devices.extend(self.bluetooth_available_devices());
        devices.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at).then_with(|| a.device_id.cmp(&b.device_id)));
        devices
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn bluetooth_available_devices(&self) -> Vec<DiscoveredDevice> {
        use crate::services::bluetooth_channel_service::fingerprint_of;
        let mine = fingerprint_of(&self.identity.device_id);
        let now = utc_now();
        let paired: Vec<(String, String)> = self
            .paired_devices()
            .unwrap_or_default()
            .into_iter()
            .map(|device| (device.peer_device_id, device.display_name))
            .collect();
        self.bluetooth
            .scanner
            .available
            .in_add_device(mine)
            .into_iter()
            .map(|device| {
                let code: String = device.fingerprint.iter().map(|byte| format!("{byte:02x}")).collect();
                let paired_here = paired.iter().find(|(id, _)| fingerprint_of(id) == device.fingerprint).cloned();
                let (device_id, hostname) = match device.identified.or(paired_here) {
                    Some(found) => found,
                    None => (format!("bt:{code}"), device.name.unwrap_or_else(|| format!("Fini device {}", &code[..4]))),
                };
                DiscoveredDevice {
                    device_id,
                    hostname,
                    addr: device.address,
                    discovery_port: 0,
                    ws_port: None,
                    last_seen_at: now.clone(),
                    channel_kind: ChannelKind::Bluetooth,
                }
            })
            .collect()
    }

    /// Pair pressed on a Bluetooth device the list shows from its
    /// advertisement: one plain link to it, this device's `IdentifyDevice`,
    /// and its `DeviceIdentified` answer -- device id, name and iroh key.
    /// `device_id` is a `bt:<fingerprint>` row, or a device id.
    pub async fn identify_bluetooth_device(&self, device_id: &str) -> Result<DiscoveredDevice, String> {
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = device_id;
            Err("Bluetooth is not available on this platform".to_string())
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            #[cfg(target_os = "android")]
            if !crate::services::ble_plugin::permission_granted() {
                return Err("Bluetooth permission required -- grant it in the dialog, then try again".to_string());
            }
            let fingerprint: [u8; 4] = match device_id.strip_prefix("bt:") {
                Some(code) => {
                    let bytes: Vec<u8> = (0..code.len())
                        .step_by(2)
                        .map(|at| u8::from_str_radix(code.get(at..at + 2).unwrap_or("zz"), 16))
                        .collect::<Result<_, _>>()
                        .map_err(|_| format!("not a Bluetooth list id: {device_id}"))?;
                    bytes.try_into().map_err(|_| format!("not a Bluetooth list id: {device_id}"))?
                }
                None => crate::services::bluetooth_channel_service::fingerprint_of(device_id),
            };
            let device = self.identify_heard_device(fingerprint).await?;
            Ok(DiscoveredDevice {
                device_id: device.device_id,
                hostname: device.hostname,
                addr: device.address,
                discovery_port: 0,
                ws_port: None,
                last_seen_at: utc_now(),
                channel_kind: ChannelKind::Bluetooth,
            })
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    async fn identify_heard_device(
        &self,
        fingerprint: [u8; crate::services::bluetooth_channel_service::FINGERPRINT_LEN],
    ) -> Result<crate::services::bluetooth_channel_service::AvailableDevice, String> {
        self.close_held_bluetooth_session_to(fingerprint).await;
        let scanner = &self.bluetooth.scanner;
        let started = std::time::Instant::now();
        let (mut link, address, _guards) = scanner
            .connect_heard_device(fingerprint)
            .await
            .inspect_err(|err| log::info!("[pairing][ble] identify failed after {:?}: {err}", started.elapsed()))?;
        let identify = PeerFrame::IdentifyDevice {
            device_id: self.identity.device_id.clone(),
            hostname: self.identity.hostname.clone(),
            endpoint_id: self.identity.endpoint_id.clone(),
        };
        send_frame(link.as_mut(), &identify).await?;
        let reply = tokio::time::timeout(IDENTIFY_ANSWER_TIMEOUT, recv_frame(link.as_mut())).await;
        log::info!("[pairing][ble] identify {address} took {:?}", started.elapsed());
        match reply {
            Ok(Some(Ok(PeerFrame::DeviceIdentified { device_id, hostname, endpoint_id }))) => {
                if device_id == self.identity.device_id {
                    return Err("that is this device".to_string());
                }
                let device = crate::services::bluetooth_channel_service::AvailableDevice { address, device_id, hostname };
                scanner.available.record_identified(&device, endpoint_id);
                Ok(device)
            }
            _ => Err("the device did not answer -- make sure Add Device is open on it".to_string()),
        }
    }

    /// Ends a held Bluetooth session to the paired device that advertises
    /// `fingerprint`, and its BLE link. While one is held, BlueZ rejects a
    /// second LE connection to the same device, so the identify step for
    /// adding it again could never connect.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    async fn close_held_bluetooth_session_to(&self, fingerprint: [u8; crate::services::bluetooth_channel_service::FINGERPRINT_LEN]) {
        use crate::services::bluetooth_channel_service::{fingerprint_of, TIME_BEFORE_LINK_CLOSE};
        let paired = self.with_conn(|conn| DeviceRepository::new(conn).ids());
        let Some(peer) =
            paired.into_iter().find(|peer| fingerprint_of(peer) == fingerprint && self.bluetooth.has_session(peer))
        else {
            return;
        };
        log::info!("[pairing][ble] closing the held Bluetooth session with {peer} to identify it again");
        self.bluetooth.close_session(&peer);
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        while self.bluetooth.has_session(&peer) && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if let Some(peer_key) = self.pinned_key(&peer).and_then(|key| key.parse::<iroh::EndpointId>().ok()) {
            self.bluetooth.close_links_to(peer_key, TIME_BEFORE_LINK_CLOSE).await;
        }
    }

    // ---- Pairing ----

    /// A `PairStart` from this device to `to_device_id`.
    fn pair_request_payload(&self, request_id: String, to_device_id: String) -> PairRequestPayload {
        let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        PairRequestPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_start".to_string(),
            request_id,
            from_device_id: self.identity.device_id.clone(),
            from_endpoint_id: Some(self.identity.endpoint_id.clone()),
            from_hostname: self.identity.hostname.clone(),
            from_discovery_port: Some(DISCOVERY_PORT),
            from_ws_port: Some(self.network.port()),
            to_device_id,
            created_at: utc_now(),
            expires_at,
        }
    }

    fn count_sent(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.tx_count += 1;
        }
    }

    /// Sends one frame before a session over the Network channel: one iroh
    /// connection to the device holding `peer_key`.
    fn send_over_network(&self, peer_key: &str, addr: IpAddr, port: u16, frame: PeerFrame) -> Result<(), String> {
        tauri::async_runtime::block_on(self.network.send_one_frame(peer_key, addr, port, &frame))
    }

    /// Sends a pair request over the Network channel.
    pub fn send_pair_request(&self, input: DevicePairRequestInput) -> Result<(), String> {
        let target_ip: IpAddr =
            input.to_addr.parse().map_err(|err| format!("invalid peer addr '{}': {err}", input.to_addr))?;
        let payload = self.pair_request_payload(input.request_id, input.to_device_id);
        let target_port = input.to_ws_port.unwrap_or(self.network.port());
        let peer_key = input
            .to_endpoint_id
            .or_else(|| self.network.heard_address(&payload.to_device_id).map(|(_, key)| key))
            .ok_or_else(|| "this device has not announced its key yet; try again in a moment, or pass its key".to_string())?;
        self.send_over_network(&peer_key, target_ip, target_port, PeerFrame::PairStart(payload.clone()))?;
        self.count_sent();
        log::info!(
            "[device-sync] pair request {} sent to {} ({target_ip}:{target_port})",
            payload.request_id,
            payload.to_device_id
        );
        Ok(())
    }

    /// Sends a pair request over Bluetooth, to the key the device's
    /// identify answer reported, and keeps that one connection open for the
    /// accept and the complete. Nothing connects back to this device:
    /// Android connects out from a private address it never advertises on.
    pub fn send_pair_request_bluetooth(&self, input: DevicePairRequestBluetoothInput) -> Result<(), String> {
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = input;
            Err("Bluetooth is not available on this platform".to_string())
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let payload = self.pair_request_payload(input.request_id, input.to_device_id);
            let to_device_id = payload.to_device_id.clone();
            let available = &self.bluetooth.scanner.available;
            let peer_key = available
                .key(&to_device_id)
                .ok_or_else(|| "that device has not been found over Bluetooth yet -- try again".to_string())?;
            // The list's address can be old: Android advertises from a new
            // private address on every Add Device change.
            let address = available.latest_address(&to_device_id).unwrap_or(input.to_bluetooth_address);
            let frame = PeerFrame::PairStart(payload.clone());
            let link = tauri::async_runtime::block_on(async {
                let (mut link, _clear_of_scan) = self.bluetooth.connect_for_pairing(&peer_key, &address).await?;
                send_pair_frame(link.as_mut(), &frame).await?;
                Ok::<_, String>(link)
            })?;
            self.listen_for_answers(link, to_device_id.clone(), address.clone());
            self.count_sent();
            log::info!("[device-sync] pair request {} sent to {to_device_id} (bluetooth {address})", payload.request_id);
            Ok(())
        }
    }

    /// Reads the accept and the complete that come back on a Bluetooth
    /// request's link.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn listen_for_answers(&self, mut link: Box<dyn DataLink>, to_device_id: String, address: String) {
        let service = self.arc();
        tauri::async_runtime::spawn(async move {
            // The whole exchange runs on this link: keep the Add Device scan
            // from connecting to the peer a second time until it is over.
            let _step = service.bluetooth.scanner.begin_pairing_step();
            let peer_key = link.peer_key().and_then(|key| key.parse::<iroh::EndpointId>().ok());
            let listen_for = Duration::from_secs(PAIR_REQUEST_TTL_SECS as u64 + 30);
            let mut completed = false;
            let _ = tokio::time::timeout(listen_for, async {
                while let Some(Ok(frame)) = recv_frame(link.as_mut()).await {
                    match frame {
                        PeerFrame::PairAccept(payload) => {
                            log::info!("[pairing][ble] accept arrived on the request's link");
                            service.receive_pair_accept(payload);
                        }
                        PeerFrame::PairComplete(payload) => {
                            log::info!("[pairing][ble] complete arrived on the request's link");
                            let from_addr = link.peer_addr().unwrap_or_default();
                            let key = link.peer_key().or_else(|| payload.from_endpoint_id.clone());
                            service.receive_pair_complete(payload, from_addr, key, true);
                            completed = true;
                            break;
                        }
                        _ => log::warn!("[pairing][ble] unexpected frame on the request's link"),
                    }
                }
            })
            .await;
            drop(link);
            log::info!("[pairing][ble] request link closed");
            let Some(peer_key) = peer_key else {
                return;
            };
            if completed {
                // The first session goes over this pairing's BLE link, which
                // is still up: no search and no new connect.
                service.bluetooth.keep_pairing_link(&to_device_id, &address, peer_key);
            } else {
                // The phone leaves Add Device with a new address; the BLE
                // link to its old one must not outlive the pairing.
                service
                    .bluetooth
                    .close_links_to(peer_key, crate::services::bluetooth_channel_service::TIME_BEFORE_LINK_CLOSE)
                    .await;
            }
        });
    }

    /// The pair requests that arrived and wait for an answer, newest first.
    pub fn incoming_requests(&self) -> Result<Vec<IncomingPairRequest>, String> {
        let mut state = self.lock()?;
        state.prune_expired_requests();
        let mut requests: Vec<IncomingPairRequest> =
            state.incoming_requests.values().map(|stored| stored.request.clone()).collect();
        requests.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.request_id.cmp(&b.request_id)));
        Ok(requests)
    }

    /// The codes of this device's own requests that the other device
    /// accepted, newest first.
    pub fn outgoing_updates(&self) -> Result<Vec<PairCodeUpdate>, String> {
        let state = self.lock()?;
        let mut updates: Vec<PairCodeUpdate> = state.outgoing_code_updates.values().cloned().collect();
        updates.sort_by(|a, b| b.accepted_at.cmp(&a.accepted_at).then_with(|| a.request_id.cmp(&b.request_id)));
        Ok(updates)
    }

    /// This device's own requests that the other device completed, newest
    /// first.
    pub fn outgoing_completions(&self) -> Result<Vec<PairCompletionUpdate>, String> {
        let state = self.lock()?;
        let mut updates: Vec<PairCompletionUpdate> = state.outgoing_pair_completions.values().cloned().collect();
        updates.sort_by(|a, b| b.paired_at.cmp(&a.paired_at).then_with(|| a.request_id.cmp(&b.request_id)));
        Ok(updates)
    }

    /// The request `request_id`, if it is still there and not expired.
    fn incoming_request(&self, request_id: &str) -> Result<StoredPairRequest, String> {
        let mut state = self.lock()?;
        state.prune_expired_requests();
        state.incoming_requests.get(request_id).cloned().ok_or_else(|| "incoming request not found".to_string())
    }

    /// Sends an answer to an incoming request, over the channel it came on.
    /// `more`: another answer to the same request follows (the accept, then
    /// the complete).
    fn answer(&self, stored: &StoredPairRequest, frame: PeerFrame, more: bool) -> Result<(), String> {
        if stored.request.via_bluetooth {
            return self.answer_over_bluetooth(stored, frame, more);
        }
        let target_ip: IpAddr =
            stored.from_addr.parse().map_err(|err| format!("invalid sender addr '{}': {err}", stored.from_addr))?;
        let peer_key = stored.from_endpoint_id.as_deref().ok_or_else(|| "the requester's key is unknown".to_string())?;
        self.send_over_network(peer_key, target_ip, stored.from_ws_port.unwrap_or(self.network.port()), frame)
    }

    /// Answers a Bluetooth `PairStart` on the connection it arrived on,
    /// which the link handler parked. With none, or a dead one, it connects
    /// to the requester's key.
    fn answer_over_bluetooth(&self, stored: &StoredPairRequest, frame: PeerFrame, more: bool) -> Result<(), String> {
        let request_id = stored.request.request_id.clone();
        let parked = self.parked_links.lock().ok().and_then(|mut links| links.remove(&request_id));
        match parked {
            Some(mut link) => {
                let sent = tauri::async_runtime::block_on(tokio::time::timeout(
                    Duration::from_secs(10),
                    send_frame(link.as_mut(), &frame),
                ));
                match sent {
                    Ok(Ok(())) => {
                        log::info!("[pairing][ble] answered {request_id} on the request's link");
                        if more {
                            if let Ok(mut links) = self.parked_links.lock() {
                                links.insert(request_id, link);
                            }
                        } else {
                            // A send only queues: hold the link until the
                            // requester closes it after it read the frame.
                            tauri::async_runtime::block_on(async {
                                let _ = tokio::time::timeout(Duration::from_secs(15), link.recv()).await;
                            });
                        }
                        return Ok(());
                    }
                    Ok(Err(err)) => log::warn!("[pairing][ble] request link failed: {err}; connecting instead"),
                    Err(_) => log::warn!("[pairing][ble] request link timed out; connecting instead"),
                }
            }
            None => log::warn!("[pairing][ble] no parked link for {request_id}; connecting instead"),
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let peer_key = stored.from_endpoint_id.as_deref().ok_or_else(|| "the requester's key is unknown".to_string())?;
            tauri::async_runtime::block_on(async {
                let (mut link, _clear_of_scan) = self.bluetooth.connect_for_pairing(peer_key, &stored.from_addr).await?;
                send_pair_frame(link.as_mut(), &frame).await
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        Err("Bluetooth is not available on this platform".to_string())
    }

    pub fn accept_pair_request(&self, input: DevicePairRequestAckInput) -> Result<PairCodeUpdate, String> {
        let stored = self.incoming_request(&input.request_id)?;
        let update = PairCodeUpdate { request_id: input.request_id, code: generate_passcode(), accepted_at: utc_now() };
        let payload = PairAcceptPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_accept".to_string(),
            request_id: update.request_id.clone(),
            code: update.code.clone(),
            from_device_id: self.identity.device_id.clone(),
            from_endpoint_id: Some(self.identity.endpoint_id.clone()),
            to_device_id: stored.request.from_device_id.clone(),
            accepted_at: update.accepted_at.clone(),
        };
        // The complete follows on the same request.
        self.answer(&stored, PeerFrame::PairAccept(payload), true)?;
        self.count_sent();
        log::info!("[device-sync] accepted request {} for {}", update.request_id, stored.request.from_device_id);
        Ok(update)
    }

    pub fn complete_pair_request(&self, input: DevicePairRequestAckInput) -> Result<(), String> {
        let stored = self.incoming_request(&input.request_id)?;
        let payload = PairCompletePayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_complete".to_string(),
            request_id: input.request_id.clone(),
            from_device_id: self.identity.device_id.clone(),
            from_endpoint_id: Some(self.identity.endpoint_id.clone()),
            from_hostname: self.identity.hostname.clone(),
            to_device_id: stored.request.from_device_id.clone(),
            paired_at: utc_now(),
            // Sent whatever channel carries this frame, so that a Network
            // completion still gives the requester a Bluetooth address.
            bluetooth_address: tauri::async_runtime::block_on(local_bluetooth_address()),
            key_material: None,
        };
        self.answer(&stored, PeerFrame::PairComplete(payload), false)?;
        {
            let mut state = self.lock()?;
            state.tx_count += 1;
            state.incoming_requests.remove(&input.request_id);
            if let Some(key) = stored.from_endpoint_id.clone() {
                state.pairing_keys.insert(input.request_id.clone(), (stored.request.from_device_id.clone(), key));
            }
        }
        self.bus.publish(Event::PairRequestsChanged);
        log::info!("[device-sync] completed request {} for {}", input.request_id, stored.request.from_device_id);
        Ok(())
    }

    pub fn acknowledge_pair_request(&self, input: DevicePairRequestAckInput) -> Result<(), String> {
        self.lock()?.incoming_requests.remove(&input.request_id);
        self.bus.publish(Event::PairRequestsChanged);
        Ok(())
    }

    fn receive_pair_request(&self, payload: PairRequestPayload, from_addr: String, from_key: Option<String>, via_bluetooth: bool) {
        if payload.to_device_id != self.identity.device_id {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.rx_count += 1;
        if !state.add_mode_enabled {
            return;
        }
        let is_new = !state.incoming_requests.contains_key(&payload.request_id);
        state.incoming_requests.insert(
            payload.request_id.clone(),
            StoredPairRequest::from_payload(&payload, from_addr, from_key, via_bluetooth),
        );
        drop(state);
        if is_new {
            log::info!(
                "[device-sync] incoming pair request {} from {} ({})",
                payload.request_id,
                payload.from_hostname,
                payload.from_device_id
            );
        }
        self.bus.publish(Event::PairRequestsChanged);
    }

    fn receive_pair_accept(&self, payload: PairAcceptPayload) {
        if payload.to_device_id != self.identity.device_id {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.rx_count += 1;
            state.outgoing_code_updates.insert(
                payload.request_id.clone(),
                PairCodeUpdate { request_id: payload.request_id, code: payload.code, accepted_at: payload.accepted_at },
            );
        }
        self.bus.publish(Event::PairRequestsChanged);
    }

    fn receive_pair_complete(&self, payload: PairCompletePayload, from_addr: String, from_key: Option<String>, via_bluetooth: bool) {
        if payload.to_device_id != self.identity.device_id {
            return;
        }
        // Over Bluetooth, the address seen on this connection wins over the
        // one the sender reports.
        let bluetooth_address = if via_bluetooth { Some(from_addr) } else { payload.bluetooth_address.clone() };
        if let Ok(mut state) = self.state.lock() {
            state.rx_count += 1;
            if let Some(key) = from_key {
                state.pairing_keys.insert(payload.request_id.clone(), (payload.from_device_id.clone(), key));
            }
            state.outgoing_pair_completions.insert(
                payload.request_id.clone(),
                PairCompletionUpdate {
                    request_id: payload.request_id,
                    from_device_id: payload.from_device_id,
                    from_hostname: payload.from_hostname,
                    paired_at: payload.paired_at,
                    via_bluetooth,
                    bluetooth_address,
                },
            );
        }
        self.bus.publish(Event::PairRequestsChanged);
    }

    /// The key to pin for `peer_device_id` once pairing `request_id` is
    /// saved: the one that came with that pairing, if it was for this peer.
    fn pairing_key(&self, request_id: &str, peer_device_id: &str) -> Option<String> {
        let state = self.state.lock().ok()?;
        let (device_id, key) = state.pairing_keys.get(request_id)?;
        (device_id == peer_device_id).then(|| key.clone())
    }

    /// A summary of the pairing state, for diagnostics.
    pub fn debug_status(&self) -> Result<DeviceConnectionDebugStatus, String> {
        let state = self.lock()?;
        Ok(DeviceConnectionDebugStatus {
            add_mode_enabled: state.add_mode_enabled,
            worker_started: self.network.discovery_started(),
            tx_count: state.tx_count,
            rx_count: state.rx_count,
            discovered_count: 0,
            peer_session_count: 0,
            incoming_request_count: state.incoming_requests.len(),
            incoming_space_mapping_update_count: 0,
            outgoing_code_count: state.outgoing_code_updates.len(),
            last_broadcast_at: None,
            last_error: None,
            discovery_port: DISCOVERY_PORT,
            discovery_provider: "iroh-mdns".to_string(),
        })
    }

    // ---- Paired devices ----

    pub fn paired_devices(&self) -> Result<Vec<Device>, String> {
        self.with_conn(|conn| DeviceRepository::new(conn).list())
    }

    fn pinned_key(&self, device_id: &str) -> Option<String> {
        self.with_conn(|conn| DeviceRepository::new(conn).pinned_key(device_id))
    }

    /// Saves a pair at the end of pairing. The key pinned is the one the
    /// peer proved during pairing `request_id`; a devtools build also takes
    /// one from the caller, so that an e2e run can make a pair the way a
    /// pairing would. The channel that the pairing ran on is set up and on
    /// (ADR-0007): a person who chose Bluetooth does not find Network set up
    /// too.
    pub fn save_paired_device(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        display_name: &str,
        bluetooth_address: Option<String>,
        via_bluetooth: bool,
        request_id: Option<String>,
        endpoint_id: Option<String>,
    ) -> Result<Device, String> {
        #[cfg(not(feature = "devtools"))]
        let endpoint_id: Option<String> = {
            drop(endpoint_id);
            None
        };
        let endpoint_id = request_id.and_then(|request_id| self.pairing_key(&request_id, peer_device_id)).or(endpoint_id);
        let device = DeviceRepository::new(conn).save(peer_device_id, display_name, endpoint_id.as_deref())?;
        let address = bluetooth_address.as_deref().and_then(normalize_bluetooth_address);
        let mut channels = ChannelRepository::new(conn);
        if via_bluetooth {
            channels.configure(peer_device_id, ChannelKind::Bluetooth, true, address.as_deref())?;
        } else {
            channels.configure(peer_device_id, ChannelKind::Network, true, None)?;
            // A Network pairing that carries an address is no proof that
            // Bluetooth works between the two: record it only.
            if let Some(address) = address.as_deref() {
                channels.note_address(peer_device_id, ChannelKind::Bluetooth, address);
            }
        }
        self.bus.publish(Event::PairedDevicesChanged);
        self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
        Ok(device)
    }

    /// Removes the pair. Its sessions end (SpaceSync hears the change), and
    /// its BLE links close.
    pub fn unpair(&self, conn: &mut SqliteConnection, peer_device_id: &str) -> Result<(), String> {
        let peer_key = DeviceRepository::new(conn).pinned_key(peer_device_id);
        DeviceRepository::new(conn).remove(peer_device_id)?;
        self.bus.publish(Event::PairedDevicesChanged);
        // Failed attempts with the old pair must not delay a new one.
        self.network.forget_failures(peer_device_id);
        self.bluetooth.forget_failures(peer_device_id);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(peer_key) = peer_key.and_then(|key| key.parse::<iroh::EndpointId>().ok()) {
            let bluetooth = self.bluetooth.clone();
            tauri::async_runtime::spawn(async move {
                bluetooth
                    .close_links_to(peer_key, crate::services::bluetooth_channel_service::TIME_BEFORE_LINK_CLOSE)
                    .await;
            });
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = peer_key;
        self.bluetooth.refresh_advertising();
        Ok(())
    }

    pub fn update_last_seen(&self, conn: &mut SqliteConnection, peer_device_id: &str, last_seen_at: &str) -> Result<(), String> {
        DeviceRepository::new(conn).set_last_seen(peer_device_id, last_seen_at)
    }

    // ---- Channels of a pair ----

    /// Every channel row of this pair (ADR-0008 D19), Network first, so
    /// rows never move. `has_session` tells which channel holds a session
    /// now (green).
    pub fn channel_statuses(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        has_session: impl Fn(ChannelKind) -> bool,
    ) -> Result<Vec<ChannelStatus>, String> {
        if !DeviceRepository::new(conn).is_paired(peer_device_id) {
            return Err("paired device not found".to_string());
        }
        // A pair-level problem: no channel of the pair can connect until it
        // is paired again.
        let pair_key_missing = DeviceRepository::new(conn).pinned_key(peer_device_id).is_none();
        Ok([ChannelKind::Network, ChannelKind::Bluetooth]
            .into_iter()
            .map(|kind| {
                let channel = ChannelRepository::new(conn).find(peer_device_id, kind);
                let state = match &channel {
                    None => ChannelState::None,
                    Some(channel) if channel.enabled => ChannelState::On,
                    Some(_) => ChannelState::Off,
                };
                let primary = channel.is_some_and(|channel| channel.is_primary);
                let problem = if pair_key_missing { Some(ChannelProblem::PairKeyMissing) } else { self.channel(kind).problem() };
                ChannelStatus::new(kind, state, has_session(kind), problem, primary)
            })
            .collect())
    }

    /// The switch on a channel row. Off: the row is off, the primary is
    /// released, and the session on the channel ends (SpaceSync hears the
    /// change). On: a channel that exists, and that works on this device
    /// (ADR-0008 D6, D15); earlier failures are forgotten.
    pub async fn set_channel_enabled(&self, peer_device_id: &str, kind: ChannelKind, enabled: bool) -> Result<(), String> {
        if !self.with_conn(|conn| DeviceRepository::new(conn).is_paired(peer_device_id)) {
            return Err("paired device not found".to_string());
        }
        if enabled && kind == ChannelKind::Bluetooth {
            // An explicit switch flip: the right moment for the permission.
            #[cfg(target_os = "android")]
            {
                crate::services::ble_plugin::request_permission();
                if !crate::services::ble_plugin::permission_granted() {
                    return Err("Bluetooth permission required -- grant it in the dialog, then try again".to_string());
                }
            }
        }
        if self.with_conn(|conn| ChannelRepository::new(conn).find(peer_device_id, kind)).is_none() {
            return Err("Set the channel up first".to_string());
        }
        let channel = self.channel(kind);
        // A radio remembered as off may be on again: ask it now.
        if enabled && !channel.usable() {
            channel.probe().await;
        }
        if enabled && !channel.usable() {
            return Err(format!("{kind:?} is unavailable on this device"));
        }
        self.with_conn(|conn| ChannelRepository::new(conn).set_enabled(peer_device_id, kind, enabled))?;
        if enabled {
            channel.forget_failures(peer_device_id);
        } else {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            if kind == ChannelKind::Bluetooth {
                if let Some(peer_key) = self.pinned_key(peer_device_id).and_then(|key| key.parse::<iroh::EndpointId>().ok()) {
                    let bluetooth = self.bluetooth.clone();
                    tauri::async_runtime::spawn(async move {
                        bluetooth
                            .close_links_to(peer_key, crate::services::bluetooth_channel_service::TIME_BEFORE_LINK_CLOSE)
                            .await;
                    });
                }
            }
        }
        self.bluetooth.refresh_advertising();
        self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
        Ok(())
    }

    /// Forgets a channel: "Unlink" on the Device page (ADR-0008 D14). The
    /// pair keeps its trust, its spaces and its other channel. Rejected
    /// while the channel is on. The peer hears of it through the outbox,
    /// written with the unlink itself.
    pub fn unlink_channel(&self, conn: &mut SqliteConnection, peer_device_id: &str, kind: ChannelKind) -> Result<(), String> {
        let mut rejected = None;
        conn.transaction::<_, diesel::result::Error, _>(|conn| {
            if let Err(reason) = ChannelRepository::new(conn).unlink(peer_device_id, kind) {
                rejected = Some(reason);
                return Err(diesel::result::Error::RollbackTransaction);
            }
            OutboxRepository::keep_in(conn, peer_device_id, &PeerFrame::ChannelUnlinked { kind })
        })
        .map_err(|err| rejected.take().unwrap_or_else(|| err.to_string()))?;
        self.bluetooth.refresh_advertising();
        self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
        Ok(())
    }

    /// Chooses the channel that carries this pair's traffic, or clears the
    /// choice (ADR-0007). Rejected for a channel that is not on.
    pub fn set_primary_channel(&self, conn: &mut SqliteConnection, peer_device_id: &str, primary: Option<ChannelKind>) -> Result<(), String> {
        if !DeviceRepository::new(conn).is_paired(peer_device_id) {
            return Err("paired device not found".to_string());
        }
        if let Some(kind) = primary {
            if !ChannelRepository::new(conn).is_enabled(peer_device_id, kind) {
                return Err("Switch the channel on first".to_string());
            }
        }
        ChannelRepository::new(conn).set_primary(peer_device_id, primary)?;
        self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
        Ok(())
    }

    // ---- Channel setup (ADR-0008 D1-D4) ----
    //
    // Both people open the setup dialog, and each device runs a setup: it
    // sends `AddChannel` to the peer on that channel and accepts the peer's
    // `AddChannel`. The init is complete on a device once both halves
    // happened there. Then the person can press OK, which switches the
    // channel on for this device alone.

    /// Starts a setup of this channel with the peer, or keeps the one that
    /// runs.
    pub async fn begin_channel_setup(&self, peer_device_id: &str, kind: ChannelKind) -> Result<(), String> {
        if kind == ChannelKind::Bluetooth {
            // An explicit click: the moment Android lets us ask for the
            // Nearby Devices permission. The answer comes later, so wait.
            #[cfg(target_os = "android")]
            {
                crate::services::ble_plugin::request_permission();
                let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
                while !crate::services::ble_plugin::permission_granted() {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("Bluetooth permission was not granted".to_string());
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
            let channel = self.channel(kind);
            if !channel.available() || !channel.probe().await {
                return Err("Bluetooth is unavailable on this device".to_string());
            }
        }
        let (already_running, attempt) = {
            static NEXT_ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let mut state = self.lock()?;
            let key = (peer_device_id.to_string(), kind);
            let already_running = state.channel_setups.contains_key(&key);
            let attempt = state
                .channel_setups
                .entry(key)
                .or_insert_with(|| ChannelSetup {
                    attempt: NEXT_ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    ..ChannelSetup::default()
                })
                .attempt;
            (already_running, attempt)
        };
        // The peer must reach this device while it searches (ADR-0008 D8).
        self.channel(kind).set_setup_open(true);
        self.bus.publish(Event::ChannelSetupChanged { peer_device_id: peer_device_id.to_string(), kind });
        if !already_running {
            let service = self.arc();
            let peer_device_id = peer_device_id.to_string();
            tauri::async_runtime::spawn(async move { service.run_channel_setup(&peer_device_id, kind, attempt).await });
        }
        Ok(())
    }

    /// The setup of this channel with the peer, if one is open.
    pub fn channel_setup(&self, peer_device_id: &str, kind: ChannelKind) -> Option<ChannelSetup> {
        self.state.lock().ok()?.channel_setups.get(&(peer_device_id.to_string(), kind)).copied()
    }

    pub fn has_channel_setup(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        self.channel_setup(peer_device_id, kind).is_some()
    }

    fn any_channel_setup(&self, kind: ChannelKind) -> bool {
        self.state.lock().is_ok_and(|state| state.channel_setups.keys().any(|(_, k)| *k == kind))
    }

    /// Records one half of an init, while `attempt` is the setup that runs.
    fn record_setup_half(&self, peer_device_id: &str, kind: ChannelKind, attempt: Option<u64>, update: impl FnOnce(&mut ChannelSetup)) {
        let changed = match self.state.lock() {
            Ok(mut state) => match state.channel_setups.get_mut(&(peer_device_id.to_string(), kind)) {
                Some(setup) if attempt.is_none_or(|attempt| setup.attempt == attempt) => {
                    update(setup);
                    true
                }
                _ => false,
            },
            Err(_) => false,
        };
        if changed {
            self.bus.publish(Event::ChannelSetupChanged { peer_device_id: peer_device_id.to_string(), kind });
        }
    }

    /// Ends the setup: OK (`switch_on`) or the dialog closed. If the init
    /// completed, the channel is made `On` for OK and `Off` for a close
    /// (ADR-0008 D15); otherwise nothing is written. A close never switches
    /// off a channel that exists.
    pub fn end_channel_setup(&self, conn: &mut SqliteConnection, peer_device_id: &str, kind: ChannelKind, switch_on: bool) -> Result<(), String> {
        let setup = self.channel_setup(peer_device_id, kind);
        let initialized = setup.is_some_and(|setup| setup.initialized());
        if initialized && (switch_on || ChannelRepository::new(conn).find(peer_device_id, kind).is_none()) {
            ChannelRepository::new(conn).configure(peer_device_id, kind, switch_on, None)?;
        }
        if let Ok(mut state) = self.state.lock() {
            state.channel_setups.remove(&(peer_device_id.to_string(), kind));
        }
        if kind == ChannelKind::Bluetooth {
            self.bluetooth.set_peer_request_accepted(peer_device_id, false);
            if initialized {
                self.bluetooth.keep_answering_after_setup();
            }
        }
        self.channel(kind).set_setup_open(self.any_channel_setup(kind));
        self.bus.publish(Event::ChannelSetupChanged { peer_device_id: peer_device_id.to_string(), kind });
        self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
        Ok(())
    }

    /// Sends `AddChannel` until the peer accepts it or this setup ends.
    async fn run_channel_setup(&self, peer_device_id: &str, kind: ChannelKind, attempt: u64) {
        // While a Bluetooth setup runs, the Add Device scan stands aside:
        // its connects to the same phone fail the setup's own.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let _clear_of_scan = (kind == ChannelKind::Bluetooth).then(|| self.bluetooth.scanner.begin_pairing_step());
        let channel = self.channel(kind);
        loop {
            match self.channel_setup(peer_device_id, kind) {
                Some(setup) if setup.attempt == attempt && !setup.sent_request_accepted => {}
                // Accepted: the setup stays open until it ends.
                Some(setup) if setup.attempt == attempt => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                _ => return,
            }
            log::info!("[setup] {kind:?} AddChannel to {peer_device_id}: connecting");
            let accepted = match channel.connect_plain(peer_device_id, SETUP_SEARCH).await {
                Ok(mut link) => {
                    let accepted = self.request_channel(link.as_mut(), peer_device_id).await;
                    log::info!("[setup] {kind:?} AddChannel to {peer_device_id}: accepted={accepted}");
                    accepted
                }
                Err(err) => {
                    log::info!("[setup] {kind:?} AddChannel to {peer_device_id}: {err}");
                    false
                }
            };
            if accepted {
                self.record_setup_half(peer_device_id, kind, Some(attempt), |setup| setup.sent_request_accepted = true);
                continue;
            }
            tokio::time::sleep(SETUP_RETRY).await;
        }
    }

    /// Sends this device's `AddChannel` on `link`. `true` if the device
    /// there is `peer_device_id` and accepted it (ADR-0008 D2).
    async fn request_channel(&self, link: &mut dyn DataLink, peer_device_id: &str) -> bool {
        let request = PeerFrame::AddChannel { device_id: self.identity.device_id.clone() };
        if send_frame(link, &request).await.is_err() {
            return false;
        }
        matches!(
            tokio::time::timeout(ACCEPT_TIMEOUT, recv_frame(link)).await,
            Ok(Some(Ok(PeerFrame::AddChannelAccept { device_id }))) if device_id == peer_device_id
        )
    }

    /// Accepts the peer's `AddChannel` on `link`, and records that this
    /// half of the init happened here.
    async fn accept_channel_request(&self, link: &mut dyn DataLink, peer_device_id: &str, kind: ChannelKind) {
        let accept = PeerFrame::AddChannelAccept { device_id: self.identity.device_id.clone() };
        if send_frame(link, &accept).await.is_ok() {
            self.record_setup_half(peer_device_id, kind, None, |setup| setup.accepted_peer_request = true);
            if kind == ChannelKind::Bluetooth {
                self.bluetooth.set_peer_request_accepted(peer_device_id, true);
            }
        }
        // Keep the link open a short time, so that the accept is read
        // before the stream closes.
        let _ = tokio::time::timeout(Duration::from_secs(1), link.recv()).await;
    }
}

#[async_trait]
impl PeerLinkHandler for DeviceService {
    /// The frames before a session: reads the first frame of a link and
    /// answers it. Discovery and pairing data are untrusted, whichever
    /// channel carried them.
    async fn accept(&self, mut link: Box<dyn DataLink>) {
        let kind = link.kind();
        let from_addr = link.peer_addr().unwrap_or_default();
        let link_key = link.peer_key();
        let Some(Ok(frame)) = recv_frame(link.as_mut()).await else {
            return;
        };
        match frame {
            PeerFrame::PairStart(payload) => {
                let key = link_key.or_else(|| payload.from_endpoint_id.clone());
                let request_id = payload.request_id.clone();
                let via_bluetooth = kind == ChannelKind::Bluetooth;
                self.receive_pair_request(payload, from_addr, key, via_bluetooth);
                if via_bluetooth {
                    log::info!("[pairing][ble] parked the link request {request_id} arrived on");
                    if let Ok(mut links) = self.parked_links.lock() {
                        links.insert(request_id.clone(), link);
                    }
                    // Hold here until the request is answered or expired:
                    // the channel counts this central as live until then.
                    // The Add Device scan stays off the adapter meanwhile.
                    #[cfg(any(target_os = "linux", target_os = "android"))]
                    let _step = self.bluetooth.scanner.begin_pairing_step();
                    let hold_until = tokio::time::Instant::now() + Duration::from_secs(PAIR_REQUEST_TTL_SECS as u64 + 5);
                    while tokio::time::Instant::now() < hold_until {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        let pending = self.state.lock().is_ok_and(|mut state| {
                            state.prune_expired_requests();
                            state.incoming_requests.contains_key(&request_id)
                        });
                        if !pending {
                            break;
                        }
                    }
                    log::info!("[pairing][ble] request {request_id} settled; releasing its link");
                    let parked = self.parked_links.lock().ok().and_then(|mut links| links.remove(&request_id));
                    drop(parked);
                }
            }
            PeerFrame::PairAccept(payload) => self.receive_pair_accept(payload),
            PeerFrame::PairComplete(payload) => {
                let key = link_key.or_else(|| payload.from_endpoint_id.clone());
                self.receive_pair_complete(payload, from_addr, key, kind == ChannelKind::Bluetooth);
            }
            // ADR-0008 D1, D2: half of a channel's init. Accepted while a
            // setup of this channel with the peer is open here, and later
            // while the channel it made exists: an accept lost on the way
            // means the peer still asks after this side finished.
            PeerFrame::AddChannel { device_id } => {
                let setup_open = self.has_channel_setup(&device_id, kind);
                let (paired, key_matches, channel_exists) = self.with_conn(|conn| {
                    let mut devices = DeviceRepository::new(conn);
                    let paired = devices.is_paired(&device_id);
                    let key_matches = devices.key_matches(&device_id, link_key.as_deref());
                    let channel_exists = ChannelRepository::new(conn).find(&device_id, kind).is_some();
                    (paired, key_matches, channel_exists)
                });
                log::info!(
                    "[setup] {kind:?} AddChannel from {device_id}: paired={paired} key={key_matches} setup_open={setup_open} channel={channel_exists}"
                );
                if paired && key_matches && (setup_open || channel_exists) {
                    self.accept_channel_request(link.as_mut(), &device_id, kind).await;
                }
            }
            // Answered only in Add Device: outside it this device is not a
            // pairing candidate.
            PeerFrame::IdentifyDevice { .. } => {
                if self.is_add_mode_enabled() {
                    let _ = send_frame(
                        link.as_mut(),
                        &PeerFrame::DeviceIdentified {
                            device_id: self.identity.device_id.clone(),
                            hostname: self.identity.hostname.clone(),
                            endpoint_id: self.identity.endpoint_id.clone(),
                        },
                    )
                    .await;
                    // Keep the link until the identifying side closes it, so
                    // that the answer is read.
                    let _ = tokio::time::timeout(Duration::from_secs(1), link.recv()).await;
                }
            }
            other => log::warn!("[pairing][accept] {kind:?} link from {from_addr}: unexpected first frame `{}`", other.wire_type()),
        }
    }
}

/// Writes one pairing frame in `PAIR_FRAME_SEND_TIMEOUT`: a stalled write
/// must not freeze the command that waits for it.
#[cfg(any(target_os = "linux", target_os = "android"))]
async fn send_pair_frame(link: &mut dyn DataLink, frame: &PeerFrame) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(10), send_frame(link, frame))
        .await
        .map_err(|_| "bluetooth pairing send timed out".to_string())?
}

/// A code of 6 digits, for the person to type on the other device.
fn generate_passcode() -> String {
    format!("{:06}", (Uuid::new_v4().as_u128() % 1_000_000) as u32)
}

/// A Bluetooth address in one form: upper case, no blanks.
pub fn normalize_bluetooth_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_uppercase())
}

/// Runs `command` with a time limit that also stops it.
async fn run_command_with_timeout(mut command: tokio::process::Command, timeout: Duration) -> Option<String> {
    command.kill_on_drop(true).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let child = command.spawn().ok()?;
    let output = tokio::time::timeout(timeout, child.wait_with_output()).await.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// This device's own Bluetooth address, where the platform can read it.
/// `None` on Android: since Android 6.0 it gives every normal app a dummy
/// value. `FINI_LOCAL_BLUETOOTH_ADDRESS` sets it for tests.
pub async fn local_bluetooth_address() -> Option<String> {
    if let Ok(value) = std::env::var("FINI_LOCAL_BLUETOOTH_ADDRESS") {
        return normalize_bluetooth_address(&value);
    }
    #[cfg(target_os = "linux")]
    {
        let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
            let mut command = tokio::process::Command::new("flatpak-spawn");
            command.arg("--host").arg("bluetoothctl");
            command
        } else {
            tokio::process::Command::new("bluetoothctl")
        };
        command.arg("show");
        let stdout = run_command_with_timeout(command, Duration::from_secs(5)).await?;
        return stdout.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("Controller ")?;
            normalize_bluetooth_address(rest.split_whitespace().next()?)
        });
    }
    #[allow(unreachable_code)]
    {
        let _ = run_command_with_timeout;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passcode_is_six_digits() {
        for _ in 0..128 {
            let code = generate_passcode();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|ch| ch.is_ascii_digit()));
        }
    }

    #[test]
    fn a_request_expires_by_this_devices_clock() {
        let payload = PairRequestPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_start".to_string(),
            request_id: "req-1".to_string(),
            from_device_id: "device-a".to_string(),
            from_hostname: "alpha".to_string(),
            from_discovery_port: None,
            from_ws_port: None,
            to_device_id: "device-b".to_string(),
            created_at: "2000-01-01T00:00:00Z".to_string(),
            expires_at: "1999-01-01T00:00:00Z".to_string(),
            from_endpoint_id: None,
        };
        let stored = StoredPairRequest::from_payload(&payload, "192.168.1.50".to_string(), None, false);
        assert!(!stored.expired(), "the sender's old expiry does not count");
        assert_eq!(stored.from_addr, "192.168.1.50");
    }
}

// ---- The data of pairing and of the channel rows ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConnectionDebugStatus {
    pub add_mode_enabled: bool,
    pub worker_started: bool,
    pub tx_count: u64,
    pub rx_count: u64,
    pub discovered_count: usize,
    pub peer_session_count: usize,
    pub incoming_request_count: usize,
    pub incoming_space_mapping_update_count: usize,
    pub outgoing_code_count: usize,
    pub last_broadcast_at: Option<String>,
    pub last_error: Option<String>,
    pub discovery_port: u16,
    pub discovery_provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingPairRequest {
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub created_at: String,
    pub expires_at: String,
    pub attempts: i64,
    pub cooldown_until: Option<String>,
    /// Whether this `PairStart` arrived over a Bluetooth link. Then
    /// `from_bluetooth_address` is the sender's address as seen on that link.
    pub via_bluetooth: bool,
    pub from_bluetooth_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCodeUpdate {
    pub request_id: String,
    pub code: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCompletionUpdate {
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub paired_at: String,
    pub via_bluetooth: bool,
    /// The completing peer's Bluetooth address, if known.
    pub bluetooth_address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestInput {
    pub request_id: String,
    pub to_device_id: String,
    pub to_addr: String,
    pub to_ws_port: Option<u16>,
    /// The peer's iroh key, for a caller that has not heard the peer on the
    /// network (a one-shot CLI command). TLS proves it either way.
    #[serde(default)]
    pub to_endpoint_id: Option<String>,
}

/// A pair request over Bluetooth: addressed by the Bluetooth address alone.
#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestBluetoothInput {
    pub request_id: String,
    pub to_device_id: String,
    pub to_bluetooth_address: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestAckInput {
    pub request_id: String,
}


/// A channel's stored state (ADR-0008 D15). `None` is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    None,
    Off,
    On,
}

/// The colour of a channel row (ADR-0008 D19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelColor {
    Green,
    Grey,
    Orange,
    Off,
    None,
}

/// One channel row on the Device page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub kind: ChannelKind,
    pub state: ChannelState,
    pub color: ChannelColor,
    /// Set only on an orange row.
    pub problem: Option<ChannelProblem>,
    /// The person's chosen channel for this pair's traffic (ADR-0007).
    pub primary: bool,
}

impl ChannelStatus {
    /// The row for a channel (ADR-0008 D19). A problem outranks a live
    /// connection: the person can only act on the problem.
    pub fn new(kind: ChannelKind, state: ChannelState, green: bool, problem: Option<ChannelProblem>, primary: bool) -> Self {
        let (color, problem) = match state {
            ChannelState::None => (ChannelColor::None, None),
            ChannelState::Off => (ChannelColor::Off, None),
            ChannelState::On => match problem {
                Some(problem) => (ChannelColor::Orange, Some(problem)),
                None if green => (ChannelColor::Green, None),
                None => (ChannelColor::Grey, None),
            },
        };
        Self { kind, state, color, problem, primary }
    }
}

#[cfg(test)]
mod channel_status_tests {
    use super::*;

    #[test]
    fn each_state_maps_to_exactly_one_row() {
        let problem = Some(ChannelProblem::BluetoothUnavailable);
        let cases = [
            (ChannelState::None, true, problem, ChannelColor::None, None),
            (ChannelState::Off, true, problem, ChannelColor::Off, None),
            (ChannelState::On, true, None, ChannelColor::Green, None),
            (ChannelState::On, false, None, ChannelColor::Grey, None),
            (ChannelState::On, true, problem, ChannelColor::Orange, problem),
            (ChannelState::On, false, problem, ChannelColor::Orange, problem),
        ];
        for (state, green, reported, color, shown) in cases {
            let row = ChannelStatus::new(ChannelKind::Bluetooth, state, green, reported, false);
            assert_eq!((row.color, row.problem), (color, shown), "{state:?}, green={green}, problem={reported:?}");
        }
    }
}


/// One channel's init in progress (ADR-0008 D1). Complete once both halves
/// are true: the peer accepted this device's `AddChannel`, and this device
/// accepted the peer's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSetup {
    /// Which setup this is. A setup closed and opened again for the same
    /// channel is a new attempt, and what the old one's task still brings
    /// back must not count for it.
    #[serde(skip)]
    pub attempt: u64,
    /// The peer accepted this device's `AddChannel`.
    pub sent_request_accepted: bool,
    /// This device accepted the peer's `AddChannel`.
    pub accepted_peer_request: bool,
}

impl ChannelSetup {
    pub fn initialized(&self) -> bool {
        self.sent_request_accepted && self.accepted_peer_request
    }
}

#[cfg(test)]
mod pair_tests {
    use super::*;
    use crate::models::Channel;
    use crate::schema::paired_devices;
    use diesel::prelude::*;
    use uuid::Uuid;

    struct Fixture {
        service: Arc<DeviceService>,
        conn: SqliteConnection,
        db_path: PathBuf,
        app_dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.db_path);
            let _ = std::fs::remove_dir_all(&self.app_dir);
        }
    }

    /// A service on a temporary database. The Network channel listens on a
    /// free port, so that the test does not meet a running app.
    fn fixture(label: &str) -> Fixture {
        let db_path = std::env::temp_dir().join(format!("fini-test-device-{label}-{}.db", Uuid::new_v4()));
        let app_dir = std::env::temp_dir().join(format!("fini-test-app-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&app_dir).unwrap();
        let mut conn = open_db_at_path(&db_path);
        let identity = DeviceRepository::new(&mut conn).load_or_create_identity(&app_dir).unwrap();
        let bus = Arc::new(EventBusService::default());
        let network = Arc::new(NetworkChannelService::on_port(identity.clone(), db_path.clone(), bus.clone(), 0));
        let bluetooth = BluetoothChannelService::new(identity.clone(), db_path.clone(), bus.clone());
        let service = DeviceService::new(identity, db_path.clone(), bus, network, bluetooth);
        Fixture { service, conn, db_path, app_dir }
    }

    /// The fixture with one paired device, `peer-a`.
    fn paired_peer(label: &str) -> Fixture {
        let mut fixture = fixture(label);
        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-a"),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(&mut fixture.conn)
            .unwrap();
        fixture
    }

    fn channel(conn: &mut SqliteConnection, peer_device_id: &str, kind: ChannelKind) -> Option<Channel> {
        ChannelRepository::new(conn).find(peer_device_id, kind)
    }

    fn save(fixture: &mut Fixture, peer_device_id: &str, address: Option<&str>, via_bluetooth: bool) {
        fixture
            .service
            .save_paired_device(&mut fixture.conn, peer_device_id, "Peer", address.map(str::to_string), via_bluetooth, None, None)
            .unwrap();
    }

    /// Regression test: `run_command_with_timeout` must end a command that
    /// outlives its deadline, not only stop waiting on it.
    #[tokio::test]
    async fn run_command_with_timeout_returns_promptly_on_a_hung_command() {
        let mut command = tokio::process::Command::new("sleep");
        command.arg("30");
        let started = std::time::Instant::now();
        let result = run_command_with_timeout(command, Duration::from_millis(200)).await;
        assert!(result.is_none());
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    /// The key a pair is saved with is the one that came with that pairing.
    /// A later frame that claims the same device id -- here a `PairStart`
    /// after the `PairComplete` -- must not replace it, or its sender could
    /// pass as the peer.
    #[test]
    fn saving_a_pair_pins_the_key_from_its_own_pairing() {
        let fixture = fixture("pin-key");
        let me = fixture.service.identity.device_id.clone();
        fixture.service.receive_pair_complete(
            PairCompletePayload {
                protocol: DISCOVERY_PROTOCOL.to_string(),
                kind: "pair_complete".to_string(),
                request_id: "req-1".to_string(),
                from_device_id: "peer-b".to_string(),
                from_hostname: "beta".to_string(),
                to_device_id: me.clone(),
                paired_at: "2026-01-01T00:00:00Z".to_string(),
                bluetooth_address: None,
                key_material: None,
                from_endpoint_id: None,
            },
            "10.0.0.2".to_string(),
            Some("legitimate-key".to_string()),
            false,
        );
        fixture.service.receive_pair_request(
            PairRequestPayload {
                protocol: DISCOVERY_PROTOCOL.to_string(),
                kind: "pair_start".to_string(),
                request_id: "req-attacker".to_string(),
                from_device_id: "peer-b".to_string(),
                from_hostname: "beta".to_string(),
                from_discovery_port: None,
                from_ws_port: None,
                to_device_id: me,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: "2026-01-01T00:01:00Z".to_string(),
                from_endpoint_id: None,
            },
            "10.0.0.66".to_string(),
            Some("attacker-key".to_string()),
            false,
        );

        assert_eq!(fixture.service.pairing_key("req-1", "peer-b").as_deref(), Some("legitimate-key"));
        // A key goes only to the device its pairing was with.
        assert_eq!(fixture.service.pairing_key("req-1", "peer-c"), None);
        assert_eq!(fixture.service.pairing_key("req-attacker", "peer-b"), None);
    }

    /// A new pair's channels follow how it was set up (ADR-0007): pairing
    /// over Bluetooth sets up Bluetooth and nothing else.
    #[test]
    fn a_bluetooth_pairing_sets_up_only_bluetooth() {
        let mut fixture = fixture("bluetooth-pairing");
        save(&mut fixture, "peer-new", Some("aa:bb:cc:dd:ee:ff"), true);

        let bluetooth = channel(&mut fixture.conn, "peer-new", ChannelKind::Bluetooth).expect("the Bluetooth channel");
        assert!(bluetooth.enabled);
        assert_eq!(bluetooth.address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert!(channel(&mut fixture.conn, "peer-new", ChannelKind::Network).is_none());
    }

    /// One side kept its row while the other reset and paired again over
    /// the network: Network must be on again here, or the pair looks set up
    /// and cannot talk.
    #[test]
    fn pairing_again_over_the_network_switches_network_on_again() {
        let mut fixture = fixture("network-repair");
        save(&mut fixture, "peer-old", None, false);
        ChannelRepository::new(&mut fixture.conn).set_enabled("peer-old", ChannelKind::Network, false).unwrap();

        save(&mut fixture, "peer-old", None, false);
        assert!(ChannelRepository::new(&mut fixture.conn).is_enabled("peer-old", ChannelKind::Network));
    }

    /// A Bluetooth address sent with a Network pairing is no proof that
    /// Bluetooth works between the two: no Bluetooth channel is set up.
    #[test]
    fn a_network_pairing_does_not_set_up_bluetooth() {
        let mut fixture = fixture("network-pairing");
        save(&mut fixture, "peer-network", Some("aa:bb:cc:dd:ee:ff"), false);

        assert!(channel(&mut fixture.conn, "peer-network", ChannelKind::Network).expect("Network").enabled);
        let bluetooth = channel(&mut fixture.conn, "peer-network", ChannelKind::Bluetooth);
        assert!(bluetooth.as_ref().is_none_or(|row| !row.enabled), "no Bluetooth channel is on: {bluetooth:?}");
    }

    /// A pairing again over Bluetooth can land on a channel that this
    /// device switched off. A whole pairing is a deliberate act: the channel
    /// is on again.
    #[test]
    fn a_bluetooth_pairing_switches_an_off_channel_on_again() {
        let mut fixture = paired_peer("bluetooth-repair");
        ChannelRepository::new(&mut fixture.conn).configure("peer-a", ChannelKind::Bluetooth, false, None).unwrap();

        save(&mut fixture, "peer-a", Some("aa:bb:cc:dd:ee:ff"), true);
        let row = channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).expect("the Bluetooth row");
        assert!(row.enabled);
        assert_eq!(row.address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    }

    /// ADR-0006: switching Bluetooth on needs neither a stored address nor
    /// an OS bond.
    #[tokio::test(flavor = "multi_thread")]
    async fn switching_a_channel_on_needs_neither_an_address_nor_a_bond() {
        let mut fixture = paired_peer("switch-on");
        // ADR-0008 D15: the channel exists (set up by an init), switched off.
        ChannelRepository::new(&mut fixture.conn).configure("peer-a", ChannelKind::Bluetooth, false, None).unwrap();
        crate::services::bluetooth_channel_service::BluetoothAdapter::pin_reachable_on_this_thread();

        fixture.service.set_channel_enabled("peer-a", ChannelKind::Bluetooth, true).await.unwrap();
        assert!(ChannelRepository::new(&mut fixture.conn).is_enabled("peer-a", ChannelKind::Bluetooth));
    }

    /// Switching a channel off is not forgetting it (ADR-0007): what it
    /// learned is kept. Forgetting is a separate act -- unlink.
    #[tokio::test(flavor = "multi_thread")]
    async fn switching_a_channel_off_keeps_what_it_learned() {
        let mut fixture = paired_peer("switch-off");
        ChannelRepository::new(&mut fixture.conn)
            .configure("peer-a", ChannelKind::Bluetooth, true, Some("AA:BB:CC:DD:EE:FF"))
            .unwrap();

        fixture.service.set_channel_enabled("peer-a", ChannelKind::Bluetooth, false).await.unwrap();
        let row = channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).expect("the row stays");
        assert!(!row.enabled);
        assert_eq!(row.address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    }

    /// Unlink is the deliberate second act: a channel that is on is not
    /// forgotten, and the refusal says what to do.
    #[test]
    fn unlink_refuses_while_the_channel_is_still_on() {
        let mut fixture = paired_peer("unlink-on");
        ChannelRepository::new(&mut fixture.conn).configure("peer-a", ChannelKind::Bluetooth, true, None).unwrap();

        let err = fixture.service.unlink_channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).unwrap_err();
        assert!(err.contains("Turn the channel off first"), "got: {err}");
        assert!(channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).is_some());
    }

    /// Once it is off, unlinking forgets it; the page offers to set it up.
    #[tokio::test(flavor = "multi_thread")]
    async fn unlink_forgets_a_switched_off_channel() {
        let mut fixture = paired_peer("unlink-off");
        ChannelRepository::new(&mut fixture.conn)
            .configure("peer-a", ChannelKind::Bluetooth, false, Some("AA:BB:CC:DD:EE:FF"))
            .unwrap();

        fixture.service.unlink_channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).unwrap();
        assert!(channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).is_none());
        let rows = fixture.service.channel_statuses(&mut fixture.conn, "peer-a", |_| false).unwrap();
        let row = rows.iter().find(|row| row.kind == ChannelKind::Bluetooth).expect("a Bluetooth row");
        assert_eq!(row.state, ChannelState::None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unpair_removes_the_pair_and_its_channels() {
        let mut fixture = paired_peer("unpair");
        ChannelRepository::new(&mut fixture.conn).configure("peer-a", ChannelKind::Bluetooth, true, None).unwrap();

        fixture.service.unpair(&mut fixture.conn, "peer-a").unwrap();
        assert!(!DeviceRepository::new(&mut fixture.conn).is_paired("peer-a"));
        assert!(
            channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).is_none(),
            "its channels go with it -- ON DELETE CASCADE"
        );
    }

    /// Unpair, then pair again: nothing is kept from the removed pair, and
    /// the new pairing sets up its channel as usual.
    #[tokio::test(flavor = "multi_thread")]
    async fn unpair_then_pair_again_over_bluetooth_starts_clean() {
        let mut fixture = paired_peer("unpair-repair");
        ChannelRepository::new(&mut fixture.conn)
            .configure("peer-a", ChannelKind::Bluetooth, true, Some("AA:BB:CC:DD:EE:FF"))
            .unwrap();
        fixture.service.unpair(&mut fixture.conn, "peer-a").unwrap();

        save(&mut fixture, "peer-a", Some("11:22:33:44:55:66"), true);
        let row = channel(&mut fixture.conn, "peer-a", ChannelKind::Bluetooth).expect("the new Bluetooth row");
        assert!(row.enabled);
        assert_eq!(row.address.as_deref(), Some("11:22:33:44:55:66"));
    }
}
