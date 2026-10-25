//! The Bluetooth channel (`BluetoothChannel`): BLE GATT `DataLink`s over
//! `ble-gatt`'s datagram tier (github.com/VRuzhentsov/ble-gatt).
//!
//! Two kinds of link share one GATT service (ADR-0009 D2, D3, D8):
//!
//! - **Sessions** (`SYNC_ALPN`) and `AddChannel` (`PEER_ALPN`) are iroh connections over
//!   `ble-gatt-iroh`, to the key pinned for the pair, which TLS proves
//!   (`BluetoothChannel::connect_session`, `BluetoothChannel::iroh_endpoint`).
//! - **Pre-pairing frames** (discovery, pair request/accept/complete) go over
//!   a plain `BleDataLink`: there is no pinned key to dial yet, so each frame
//!   carries the sender's key itself.
//!
//! An accepted channel is routed by its first datagram (`route_inbound`): a
//! QUIC Initial goes to the iroh transport, Fini's JSON to the gate.
//!
//! Linux (BlueZ via `ble_gatt::drivers::linux`) and Android (the driver
//! `tauri-plugin-ble-gatt` builds; Fini uses it from Rust, not through Tauri
//! IPC). See `adapter` for why construction is deferred, and
//! `start_peripheral_once`/its caller in `sync::commands` for why the
//! peripheral role isn't spawned from `.setup()` on Android the way it is on
//! Linux.
//!
//! ADR-0003 revision: dials/accepts unconditionally, independent of
//! Network's own state -- both transports stay connected to a paired peer
//! at once, with `preferred_transport` only deciding which one is primary
//! (see `pairing::DeviceConnectionState::recompute_primary_locked`),
//! not whether Bluetooth connects at all.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use ble_gatt::drivers::linux::LinuxDriver;
use ble_gatt::datagram::{self, DatagramChannel, DatagramConfig};
use ble_gatt::{Adapter, CharacteristicUuid, PeerAddress, ServiceUuid};
use uuid::Uuid;

use crate::models::{Event, Identity};
use crate::repositories::{ChannelRepository, DeviceRepository};
use crate::services::channel_service::iroh_link::{IrohDataLink, PEER_ALPN, SYNC_ALPN};
use crate::services::channel_service::{
    ChannelKind, ChannelProblem, ChannelService, DataLink, LinkHandlerSlot, LinkHandlers,
};
use crate::services::db::open_db_at_path;
use crate::services::event_bus_service::EventBusService;
use crate::services::space_sync_service::rejected_by_peer;


pub use adapter::BluetoothAdapter;
pub use data_link::*;

pub use advertiser::Advertiser;
pub(crate) use endpoint::TIME_BEFORE_LINK_CLOSE;
pub use scanner::Scanner;
pub use scanner::AvailableDevice;

use scanner::Purpose;

/// The Bluetooth channel. It owns the scan and what the scan heard, the
/// advertisement, the delivery retry schedule and the iroh endpoint over
/// Bluetooth.
pub struct BluetoothChannelService {
    identity: Identity,
    db_path: PathBuf,
    bus: Arc<EventBusService>,
    /// This object, for the tasks it starts.
    me: Weak<BluetoothChannelService>,
    handlers: LinkHandlerSlot,
    /// Whether a Bluetooth setup is open on this device: then it advertises
    /// (ADR-0008 D8), and the links of the peer are kept.
    setup_open: std::sync::atomic::AtomicBool,
    /// The devices whose `AddChannel` an open setup accepted
    /// (`wait_for_turn_to_connect`).
    peer_requests_accepted: std::sync::Mutex<std::collections::HashSet<String>>,
    /// The one Bluetooth scan, the turns on the radio, and what the scan
    /// heard: the devices available for Add Device and the paired devices.
    pub(crate) scanner: Arc<Scanner>,
    /// What this device advertises over Bluetooth, and whether it does.
    pub(crate) advertiser: Arc<Advertiser>,
    /// When to search again for a paired device that a delivery search did
    /// not find.
    pub(crate) delivery: DeliverySchedule,
    /// The iroh endpoint and its `ble-gatt-iroh` transport, built on first
    /// use (`iroh_endpoint`).
    endpoint: tokio::sync::OnceCell<(iroh::Endpoint, ble_gatt_iroh::BleGattTransport)>,
    /// Per paired device, the address of the BLE link a pairing just used and
    /// left up: the first session goes over it (`keep_pairing_link`).
    pairing_links: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl BluetoothChannelService {
    pub fn new(identity: Identity, db_path: PathBuf, bus: Arc<EventBusService>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            scanner: Arc::new(Scanner::new(bus.clone())),
            advertiser: Arc::new(Advertiser::new(&identity)),
            delivery: DeliverySchedule::new(bus.clone()),
            endpoint: tokio::sync::OnceCell::new(),
            pairing_links: std::sync::Mutex::default(),
            identity,
            db_path,
            bus,
            me: me.clone(),
            handlers: LinkHandlerSlot::default(),
            setup_open: std::sync::atomic::AtomicBool::new(false),
            peer_requests_accepted: std::sync::Mutex::default(),
        })
    }

    /// This object as an `Arc`, for a task it starts.
    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("the Bluetooth channel lives as long as its tasks")
    }

    /// Whether a session with the peer runs on Bluetooth now.
    pub fn has_session(&self, peer_id: &str) -> bool {
        self.handlers.has_session(peer_id, ChannelKind::Bluetooth)
    }

    /// Ends the Bluetooth session with the peer, if one runs.
    pub fn close_session(&self, peer_id: &str) -> bool {
        self.handlers.close_session(peer_id, ChannelKind::Bluetooth)
    }

    /// The iroh key pinned for the pair, if any.
    fn pinned_key(&self, peer_id: &str) -> Option<String> {
        tokio::task::block_in_place(|| DeviceRepository::new(&mut open_db_at_path(&self.db_path)).pinned_key(peer_id))
    }

    /// Whether a Bluetooth setup is open on this device.
    pub fn setup_open(&self) -> bool {
        self.setup_open.load(Ordering::SeqCst)
    }
}

/// When a delivery search does not find the device, the next one waits this
/// long, one step further each time and then the last step always: 1, 5 and
/// 15 minutes, then every 15 (ADR-0008 D12).
const DELIVERY_RETRY: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];

/// When the next delivery search is due after `misses` searches in a row
/// found nothing.
fn next_delivery_search(misses: usize, now: std::time::Instant) -> std::time::Instant {
    now + DELIVERY_RETRY[misses.saturating_sub(1).min(DELIVERY_RETRY.len() - 1)]
}

/// After the peer's gate rejects a session (its channel is off for this
/// pair), the next connection waits this long, also while the peer is heard.
/// The same wait as Network's `FAILURE_COOLDOWN`. The peer connects by
/// itself when it switches the channel on.
const REJECTED_WAIT_TIME: Duration = Duration::from_secs(30);

/// Per paired device: how many delivery searches in a row found nothing,
/// and when the next one is due; and until when a rejection stops the next
/// connection. In memory only, as ADR-0008 decided: a started app tries at
/// once.
pub struct DeliverySchedule {
    bus: Arc<EventBusService>,
    schedule: std::sync::Mutex<std::collections::HashMap<String, (usize, std::time::Instant)>>,
    rejected_until: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl DeliverySchedule {
    pub fn new(bus: Arc<EventBusService>) -> Self {
        Self { bus, schedule: Default::default(), rejected_until: Default::default() }
    }

    /// Tells the bus once `after` has passed: a search or a connection to
    /// the peer is due again (ADR-0008 D12).
    fn publish_due_after(&self, peer_id: &str, after: Duration) {
        let bus = self.bus.clone();
        let event = Event::ConnectChanged { peer_device_id: peer_id.to_string(), kind: ChannelKind::Bluetooth };
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(after).await;
            bus.publish(event);
        });
    }

    /// Whether a delivery search for this device is due. A device that
    /// rejected a session recently is not. A device heard recently is
    /// otherwise always due: it is reached by one connection.
    pub fn delivery_due(&self, scanner: &Scanner, peer_id: &str) -> bool {
        let rejected = self
            .rejected_until
            .lock()
            .is_ok_and(|rejected| rejected.get(peer_id).is_some_and(|until| std::time::Instant::now() < *until));
        if rejected {
            return false;
        }
        if scanner.heard_recently(peer_id) {
            return true;
        }
        match self.schedule.lock() {
            Ok(schedule) => schedule
                .get(peer_id)
                .is_none_or(|(_, next)| std::time::Instant::now() >= *next),
            Err(_) => true,
        }
    }

    pub fn note_delivery_missed(&self, peer_id: &str) {
        let now = std::time::Instant::now();
        let next = match self.schedule.lock() {
            Ok(mut schedule) => {
                let misses = schedule.get(peer_id).map_or(0, |(misses, _)| *misses) + 1;
                let next = next_delivery_search(misses, now);
                schedule.insert(peer_id.to_string(), (misses, next));
                next
            }
            Err(_) => return,
        };
        self.publish_due_after(peer_id, next.saturating_duration_since(now));
    }

    /// The peer rejected a session: the next connection waits
    /// `REJECTED_WAIT_TIME`.
    pub fn record_rejected(&self, peer_id: &str) {
        if let Ok(mut rejected) = self.rejected_until.lock() {
            rejected.insert(peer_id.to_string(), std::time::Instant::now() + REJECTED_WAIT_TIME);
        }
        self.publish_due_after(peer_id, REJECTED_WAIT_TIME);
    }

    /// The device was reached, or see `ChannelService::forget_failures`: the
    /// next delivery search is due at once. Also used when a pair is removed.
    pub fn forget_delivery_misses(&self, peer_id: &str) {
        if let Ok(mut schedule) = self.schedule.lock() {
            schedule.remove(peer_id);
        }
        if let Ok(mut rejected) = self.rejected_until.lock() {
            rejected.remove(peer_id);
        }
    }
}

/// Fini's own GATT service/characteristic for the datagram tier. Fixed, not
/// user-configurable: both sync peers must advertise/expect the same UUIDs
/// to find each other's service. Distinct from any third-party device's own
/// UUIDs — this is Fini-to-Fini only, the app-to-app case `ble-gatt`'s
/// ADR-0003 describes.
const FINI_BLE_SERVICE_UUID: &str = "b1e6a000-f101-4000-8000-00805f9b34fb";
const FINI_BLE_CHARACTERISTIC_UUID: &str = "b1e6a001-f101-4000-8000-00805f9b34fb";

/// Fini's service and characteristic, for dialling and scanning. What this
/// device advertises adds its identity: `Advertiser::advertised_config`.
fn datagram_config() -> DatagramConfig {
    DatagramConfig::new(
        ServiceUuid(Uuid::parse_str(FINI_BLE_SERVICE_UUID).expect("valid UUID literal")),
        CharacteristicUuid(Uuid::parse_str(FINI_BLE_CHARACTERISTIC_UUID).expect("valid UUID literal")),
    )
}

/// Four bytes of FNV-1a over the `device_id`.
///
/// FNV-1a specifically, and **not** `std::collections::hash_map::DefaultHasher`:
/// that one's output is explicitly not guaranteed stable across Rust
/// releases, so two peers built with different toolchains would compute
/// different fingerprints for the same device and never recognise each
/// other. FNV-1a is a fixed, published algorithm, so the value is stable
/// for as long as the `device_id` is.
///
/// Not a security boundary and not meant to be one: the TLS handshake
/// against the pinned key is, and this only decides which advertiser is
/// worth dialling. A collision costs one wasted dial that the handshake
/// then fails.
///
/// ADR-0006 records the privacy limit this carries: a stable value
/// broadcast continuously is trackable, which is why the rotating,
/// secret-derived form is the intended successor once pairing establishes
/// key material (issue #162).
pub fn fingerprint_of(device_id: &str) -> [u8; FINGERPRINT_LEN] {
    const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in device_id.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash.to_be_bytes()
}

/// Reads the fingerprint out of a discovered peer's manufacturer data, or
/// `None` for an advertiser that carries none -- a peer running a build
/// from before ADR-0006's slice 2, which must stay dialable rather than
/// being filtered out for being old.
fn advertised_fingerprint(manufacturer_payload: Option<&[u8]>) -> Option<[u8; FINGERPRINT_LEN]> {
    let payload = manufacturer_payload?;
    payload.get(1..1 + FINGERPRINT_LEN)?.try_into().ok()
}

pub(crate) const FINGERPRINT_LEN: usize = 4;

/// `0xFFFF` is the Bluetooth SIG's own reserved value for "manufacturer
/// specific data" used for testing and non-market purposes — the
/// appropriate choice for a private, unregistered app like Fini rather
/// than picking an arbitrary value that could collide with a real vendor's
/// company ID some other nearby scanner is specifically watching for.
const FINI_MANUFACTURER_ID: u16 = 0xFFFF;
/// Bit 0 of the manufacturer payload's first byte: whether this device is
/// currently in add-mode.
///
/// The payload used to be exactly this one byte, present only while in
/// add-mode. ADR-0006's slice 2 made it a five-byte record — one flags
/// byte then four fingerprint bytes — advertised always, because the
/// fingerprint is how a scanner tells which peer it has found. Five bytes
/// still fits: legacy advertisements cap at 31 total and the 128-bit
/// service UUID plus this record spend about 26. See
/// `GattServiceSpec::manufacturer_data`'s own doc comment in ble-gatt.
const ADD_MODE_FLAG_BYTE: u8 = 0x01;

/// The Bluetooth half of the held connections (2026-10-13: green is a live
/// connection):
///
/// - **Green**: a held session with the peer (`is_green`).
/// - **Search** is one coordinator (`Scanner`, ADR-0008 D13) serving every
///   purpose with a single scan:
///   - delivery: a paired device with Bluetooth on and no session is
///     searched for up to `SEARCH_WINDOW` and connected to as soon as it is
///     heard; a device not found is tried again after `DELIVERY_RETRY`
///     steps, until it appears;
///   - setup, while the setup dialog is open, finds the peer for the hello.
/// - **Session**: the held link goes to `SpaceSync::hold_session`.
///
/// Nothing scans when there is no reason to: every session held, no setup
/// -- the radio only advertises and listens for connections.
const SEARCH_WINDOW: Duration = Duration::from_secs(60);

/// Per-candidate cap for one connect inside a delivery search: an
/// advertiser that accepts the connection and never finishes the handshake
/// must not hold the whole window.
const CONNECT_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one scan window listens before the wanted devices are read
/// again.
const SCAN_WINDOW: Duration = Duration::from_secs(20);

#[async_trait]
impl ChannelService for BluetoothChannelService {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn set_link_handlers(&self, handlers: LinkHandlers) {
        self.handlers.set(handlers);
    }

    fn set_add_device_mode(&self, on: bool) {
        self.advertiser.set_add_mode(on);
        self.scanner.set_add_device_mode(on);
        self.refresh_advertising();
    }

    fn set_setup_open(&self, open: bool) {
        self.setup_open.store(open, Ordering::SeqCst);
        self.refresh_advertising();
    }

    /// Linux and Android: the only builds with a Bluetooth channel.
    fn available(&self) -> bool {
        true
    }

    fn usable(&self) -> bool {
        !BluetoothAdapter::this_device().is_unavailable()
    }

    async fn probe(&self) -> bool {
        BluetoothAdapter::this_device().probe(&self.scanner).await
    }

    fn start_serving(self: Arc<Self>) {
        // Linux only, deliberately. Android starts the same loop from
        // `keep_serving`, once its Activity context exists -- starting it
        // here as well gave Android two accept loops racing each other.
        #[cfg(target_os = "linux")]
        tauri::async_runtime::spawn(async move { self.run_server().await });
    }

    fn keep_serving(self: Arc<Self>) {
        // Android starts its peripheral lazily, from the first real tick:
        // the Activity context it needs does not exist when the app boots
        // (see `start_peripheral_once`). Gated on the Nearby Devices
        // permission, because this runs from a background tick rather than
        // a user action -- without it `startAdvertising` throws
        // SecurityException and the loop retries forever.
        #[cfg(target_os = "android")]
        if crate::services::ble_plugin::permission_granted() {
            self.start_peripheral_once();
        }
        self.refresh_advertising();
        // Hold a session with every paired device that has Bluetooth on
        // (2026-10-13: green is a live connection). `start_exchange` skips
        // a device that has one, or whose delivery search is not due.
        let peers = tokio::task::block_in_place(|| {
            ChannelRepository::new(&mut open_db_at_path(&self.db_path)).peers_with_channel_enabled(ChannelKind::Bluetooth)
        });
        for peer in peers {
            self.start_exchange(&peer);
        }
    }

    fn forget_failures(&self, peer_device_id: &str) {
        self.delivery.forget_delivery_misses(peer_device_id);
    }

    /// A search of up to `within`, and a connection the moment the peer is
    /// heard. A search the radio refused pauses `SETUP_ROUND_FAILURE_PAUSE`
    /// before it returns, so a broken radio is not hammered.
    async fn connect_plain(&self, peer_device_id: &str, within: Duration) -> Result<Box<dyn DataLink>, String> {
        let result = self.find_and_connect_plain(peer_device_id, within).await;
        if BluetoothAdapter::this_device().is_unavailable() {
            tokio::time::sleep(SETUP_ROUND_FAILURE_PAUSE).await;
        }
        result
    }

    fn connecting(&self, peer_device_id: &str) -> bool {
        self.scanner.connecting(peer_device_id)
    }

    fn problem(&self) -> Option<ChannelProblem> {
        BluetoothAdapter::this_device()
            .is_unavailable()
            .then_some(ChannelProblem::BluetoothUnavailable)
    }
}

impl BluetoothChannelService {
    /// Starts the Bluetooth peripheral acceptor loop once. Android only: on
    /// Linux `start_serving` starts it at app start. On Android the Activity
    /// context it needs exists only after the webview loads, so the first
    /// call comes from a SpaceSync tick. The claim in `run_server` is the
    /// rule; this check only keeps the regular path quiet.
    #[cfg(target_os = "android")]
    fn start_peripheral_once(&self) {
        if !self.advertiser.peripheral_running() {
            let channel = self.arc();
            tauri::async_runtime::spawn(async move { channel.run_server().await });
        }
    }

    /// Peripheral role: advertise Fini's BLE service and gate every
    /// accepted central through the same transport-neutral session gate
    /// every other adapter uses. No-op (logs and returns) when the local
    /// adapter can't do peripheral mode, or isn't available at all —
    /// Bluetooth is always a fallback, never a hard requirement to start the
    /// app. `ui-plane`/`test` only — `cli-plane` dials out but does not run
    /// an inbound acceptor.
    pub async fn run_server(&self) {
        use futures_util::StreamExt;

        let advertiser = self.advertiser.clone();
        if !advertiser.claim_peripheral_role() {
            log::warn!("[transport][ble] peripheral acceptor already running; ignoring a second start");
            return;
        }

        // Retried with backoff, not returned-from-once: this is spawned
        // exactly once at startup, so an early failure here (adapter off,
        // BlueZ restarting, briefly unavailable) used to end the peripheral
        // role for the rest of the process's life even after the adapter
        // itself became able to retry. If this device also has the higher
        // device id, the deterministic dial rule means it never dials out
        // either — Bluetooth would be unusable until restart. Looping means
        // enabling Bluetooth later (or the adapter coming back) can still
        // stand up the acceptor.
        let mut delay = Duration::from_secs(2);
        let max_delay = Duration::from_secs(60);

        let mut advertising_wanted = advertiser.watch_wanted();
        loop {
            // ADR-0008 D8: advertise only while there is someone this device
            // should be reachable by.
            while !*advertising_wanted.borrow_and_update() {
                if advertising_wanted.changed().await.is_err() {
                    return;
                }
            }
            let bluetooth = BluetoothAdapter::this_device();
            let adapter = match bluetooth.adapter().await {
                Ok(adapter) => adapter,
                Err(err) => {
                    bluetooth.note_unreachable();
                    log::warn!("[transport][ble] adapter unavailable, retrying in {delay:?}: {err}");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(max_delay);
                    continue;
                }
            };
            if !adapter.capabilities().await.peripheral {
                log::warn!(
                    "[transport][ble] adapter has no peripheral support; retrying in {delay:?} \
                     in case a different adapter becomes available"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
                continue;
            }
            // Subscribed *before* the config snapshot/`serve` call below,
            // not after: a `watch::Receiver` only misses changes that happen
            // strictly before it subscribes, so subscribing here closes the
            // window where a toggle lands while the advertisement (built
            // from the config snapshot `serve` takes) is still starting up
            // -- a real async operation, not instant. Subscribing afterward
            // would let that specific toggle go unseen (the receiver's
            // baseline already reflects the new value at subscribe time),
            // leaving the device advertising without the add-mode flag,
            // undiscoverable, until some *later* toggle happens to fire
            // `changed()` again. Watched (not merely read) so a toggle
            // mid-serve interrupts the accept loop below immediately, rather
            // than only taking effect on whatever later triggers a natural
            // re-advertise -- see `Advertiser::add_mode`'s doc comment for
            // why this is a signal to the running loop rather than a full
            // task restart.
            let mut add_mode_rx = advertiser.watch_add_mode();
            let mut incoming = match datagram::serve(&adapter, &advertiser.advertised_config()).await {
                Ok(stream) => {
                    // Advertising is a real use of the radio, so it clears a
                    // previously-recorded failure just as a scan does.
                    bluetooth.note_reachable();
                    stream
                }
                Err(err) => {
                    bluetooth.note_unreachable();
                    log::warn!("[transport][ble] advertise failed, retrying in {delay:?}: {err}");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(max_delay);
                    continue;
                }
            };
            log::info!("[transport][ble] advertising, awaiting centrals");
            delay = Duration::from_secs(2);

            let mut restarting_for_add_mode_change = false;
            // BlueZ stops transmitting the advertisement once a central that
            // connected to it has gone, while still reporting the instance
            // as active (`ActiveInstances: 1`). Measured: the phone heard the
            // desktop's advertisement three times right after it was
            // registered, a phone-initiated hello then connected and left,
            // and the phone heard nothing for 4.5 minutes -- until the
            // advertisement was registered again. So the advertisement is
            // re-registered when the last accepted central is gone; doing it
            // earlier would tear down the live link.
            let live_centrals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (central_gone_tx, mut central_gone_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
            loop {
                tokio::select! {
                    Some(()) = central_gone_rx.recv() => {
                        // Linux only. Android keeps advertising, and
                        // registering the advertisement again gives the phone
                        // a new private address: the address the other device
                        // had just heard goes stale, and its dial hangs for
                        // 15 s before the search hears the new one.
                        if cfg!(target_os = "linux") && live_centrals.load(Ordering::SeqCst) == 0 {
                            log::info!("[transport][ble] last central gone; re-advertising");
                            restarting_for_add_mode_change = true;
                            break;
                        }
                    }
                    channel = incoming.next() => {
                        let Some(channel) = channel else { break; };
                        // Without this, a link that dies before it sends
                        // anything (lost at the GATT layer before the first
                        // read) would leave no trace that anything was
                        // accepted.
                        log::info!("[transport][ble] central connected: {}", channel.peer().0);
                        live_centrals.fetch_add(1, Ordering::SeqCst);
                        let live_centrals = live_centrals.clone();
                        let central_gone_tx = central_gone_tx.clone();
                        let bluetooth = self.arc();
                        // The central counts as live until its channel
                        // closes: the handler finishing on a plain link, or
                        // the iroh transport closing the channel of a session.
                        tokio::spawn(async move {
                            // The device connected to us is the one dialling:
                            // the add-mode scan must not dial back while it
                            // is here. A second, crossing connection to the
                            // same device is what kept dropping both
                            // (initiator = central).
                            let _step = bluetooth.scanner.begin_pairing_step();
                            if let Some(handled) = bluetooth.route_inbound(channel).await {
                                let _ = handled.await;
                            }
                            live_centrals.fetch_sub(1, Ordering::SeqCst);
                            let _ = central_gone_tx.send(());
                        });
                    }
                    _ = add_mode_rx.changed() => {
                        // Only the advertisement changes: the GATT server, and
                        // with it the links of connected centrals, stays. A
                        // new server dropped the link the first session after
                        // a pairing runs on.
                        match datagram::update_advertisement(&adapter, &advertiser.advertised_config()).await {
                            Ok(()) => log::info!("[transport][ble] add-mode changed; advertisement updated"),
                            Err(err) => {
                                log::info!("[transport][ble] add-mode changed; re-advertising ({err})");
                                restarting_for_add_mode_change = true;
                                break;
                            }
                        }
                    }
                    _ = advertising_wanted.changed() => {
                        if !*advertising_wanted.borrow() {
                            log::info!("[transport][ble] nothing to be reachable for; stopping advertising");
                            restarting_for_add_mode_change = true;
                            break;
                        }
                    }
                }
            }
            if restarting_for_add_mode_change {
                // Ending `incoming` here (by falling through to the outer
                // loop's next `datagram::serve` call) is what actually stops
                // the old advertisement -- ble-gatt's drivers tear down the
                // previous generation's GATT server/advertisement as part of
                // starting a new one (see BleGattBridge.startAdvertising's
                // own doc comment: "tear down any predecessor first").
                delay = Duration::from_secs(2);
                continue;
            }
            // The serve stream itself ended (e.g. the adapter dropped out
            // from under it) rather than a caller closing it or an add-mode
            // change — nothing else here ever drops the stream deliberately.
            // Retry rather than leaving the peripheral role dead.
            log::warn!("[transport][ble] serve stream ended unexpectedly; retrying in {delay:?}");
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(max_delay);
        }
    }

    /// Start an exchange with this peer over Bluetooth unless one is running
    /// or being attempted (ADR-0008 D10). Called when there is work for the
    /// peer.
    pub fn start_exchange(&self, peer_id: &str) {
        if self.has_session(peer_id) || !self.delivery.delivery_due(&self.scanner, peer_id) {
            return;
        }
        // While a device is being added, the adapter belongs to that search --
        // except for the device just paired over a link that is still up.
        if self.advertiser.in_add_mode() && !self.has_pairing_link(peer_id) {
            return;
        }
        if !self.scanner.begin_exchange(peer_id) {
            return;
        }
        let bluetooth = self.arc();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            bluetooth.exchange_with(&peer_id).await;
            bluetooth.scanner.end_exchange(&peer_id);
        });
    }

    /// One delivery (ADR-0008 D8, D12): a present peer is one dial to where
    /// it was last heard; otherwise a delivery search of up to
    /// `SEARCH_WINDOW` connects the moment the peer is heard. Then
    /// authenticate and run the exchange to its idle end.
    async fn exchange_with(&self, peer_id: &str) {
        if !self.is_on(peer_id) {
            return;
        }
        // Just paired over Bluetooth: the pairing's BLE link is still up, so
        // the session goes over it, with no search and no new connect.
        if let Some(address) = self.remove_pairing_link(peer_id) {
            let guard = self.scanner.acquire_connect().await;
            match self.connect_for_session(peer_id, &address).await {
                Ok(link) => {
                    drop(guard);
                    log::info!("[transport][ble] first session with {peer_id} over the pairing link");
                    return self.run_exchange(peer_id, link, &address).await;
                }
                Err(err) => log::info!("[transport][ble] the pairing link to {address} is gone: {err}"),
            }
        }
        let deadline = tokio::time::Instant::now() + SEARCH_WINDOW;
        let mut last_error = None;

        if let Some(address) = self.scanner.last_heard_address(peer_id) {
            let guard = self.scanner.acquire_connect().await;
            match self.connect_for_session(peer_id, &address).await {
                Ok(link) => {
                    drop(guard);
                    return self.run_exchange(peer_id, link, &address).await;
                }
                Err(err) => last_error = Some(err),
            }
        }

        loop {
            // The pair or the channel can go while the search runs. A search
            // that went on dialled a device that was not paired any more, and
            // its connect stopped the Add Device scan until it timed out.
            if !self.is_on(peer_id) {
                return;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let found = if remaining.is_zero() {
                None
            } else {
                self.scanner.find(peer_id, Purpose::Delivery, remaining).await
            };
            let Some(found) = found else {
                match last_error {
                    Some(err) => log::info!("[transport][ble] {peer_id} refused the exchange: {err}"),
                    None => log::info!("[transport][ble] {peer_id} not heard within the search window"),
                }
                self.delivery.note_delivery_missed(peer_id);
                return;
            };
            if !self.is_on(peer_id) {
                return;
            }
            match self.connect_for_session(peer_id, &found.address).await {
                Ok(link) => {
                    let address = found.address.clone();
                    drop(found);
                    return self.run_exchange(peer_id, link, &address).await;
                }
                Err(err) => last_error = Some(err),
            }
        }
    }

    /// Connect a session to `peer_id` at `address`, bounded so an
    /// advertiser that accepts the connection and never finishes the
    /// handshake cannot hold the search. One real `connect()` was measured
    /// at ~28s. TLS proves the peer; its gate may still refuse by closing
    /// the connection (`run_session_gate`).
    ///
    /// The connect also stops when the pair or the channel goes while it
    /// runs: a connect that went on to its timeout after an unpair stopped
    /// the Add Device scan for up to 20 s.
    async fn connect_for_session(&self, peer_id: &str, address: &str) -> Result<Box<dyn DataLink>, String> {
        let connect = tokio::time::timeout(
            CONNECT_CANDIDATE_TIMEOUT,
            self.connect_session(peer_id, address, SYNC_ALPN),
        );
        let attempt = tokio::select! {
            attempt = connect => attempt,
            () = self.wait_until_off(peer_id) => {
                log::info!("[transport][ble] stopped the connect to {address}: {peer_id} has no Bluetooth channel any more");
                return Err("the pair or the channel is gone".to_string());
            }
        };
        match attempt {
            Ok(Ok(link)) => Ok(link),
            Ok(Err(err)) => {
                log::info!("[transport][ble] candidate {address} is not {peer_id}: {err}");
                self.scanner.note_connect_failed(address);
                Err(err)
            }
            Err(_elapsed) => {
                log::info!("[transport][ble] candidate {address} did not finish the connect in time");
                self.scanner.note_connect_failed(address);
                Err("connect timed out".to_string())
            }
        }
    }

    /// Returns when the pair with `peer_id` has no Bluetooth channel that is
    /// on: the pair was removed, or the channel was switched off or removed.
    async fn wait_until_off(&self, peer_id: &str) {
        while self.is_on(peer_id) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn run_exchange(&self, peer_id: &str, link: Box<dyn DataLink>, address: &str) {
        self.delivery.forget_delivery_misses(peer_id);
        // The switch could have been turned off during the search.
        if !self.is_on(peer_id) {
            return;
        }
        let Some(handlers) = self.handlers.get().cloned() else {
            return;
        };
        // Diagnostics only: nothing connects to a stored address.
        tokio::task::block_in_place(|| {
            ChannelRepository::new(&mut open_db_at_path(&self.db_path)).note_address(
                peer_id,
                ChannelKind::Bluetooth,
                address,
            )
        });
        let held = handlers
            .session
            .hold(
                link,
                peer_id,
                ChannelKind::Bluetooth,
                Box::new(|| {
                    self.scanner.note_connected(peer_id);
                    log::info!("[transport][ble] holding a session with {peer_id} via {address}");
                }),
            )
            .await;
        // Rejected by the peer: the next connection waits
        // `REJECTED_WAIT_TIME`, though the peer is still heard.
        if matches!(held, Ok(close_code) if rejected_by_peer(close_code)) {
            self.delivery.record_rejected(peer_id);
        }
        if held.is_ok() {
            self.close_links_after_session(peer_id);
        }
    }

    /// Works out again whether to advertise, from the stored channels and
    /// the setups open now. Cheap; called whenever one of these can change.
    pub fn refresh_advertising(&self) {
        let any_channel_on = tokio::task::block_in_place(|| {
            !ChannelRepository::new(&mut open_db_at_path(&self.db_path))
                .peers_with_channel_enabled(ChannelKind::Bluetooth)
                .is_empty()
        });
        let advertiser = &self.advertiser;
        let wanted =
            any_channel_on || self.setup_open() || advertiser.answering_after_setup() || advertiser.in_add_mode();
        advertiser.set_wanted(wanted);
    }

    /// A Bluetooth setup finished: keep advertising a short time, so that
    /// the peer's late `AddChannel` is answered, then work it out again.
    pub fn keep_answering_after_setup(&self) {
        self.advertiser.keep_answering_after_setup();
        let bluetooth = self.arc();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(advertiser::ANSWER_AFTER_SETUP + Duration::from_secs(1)).await;
            bluetooth.refresh_advertising();
        });
    }
}

// ---- Channel setup ----

/// Pause after a Bluetooth round fails outright (scan refused, adapter
/// gone), so a broken radio is not hammered.
#[cfg(not(test))]
const SETUP_ROUND_FAILURE_PAUSE: Duration = Duration::from_secs(5);
#[cfg(test)]
const SETUP_ROUND_FAILURE_PAUSE: Duration = Duration::from_millis(200);

/// How long the device with the higher id waits, in a Bluetooth setup, for the
/// other device's hello to reach it before dialling anyway. See
/// `BluetoothChannel::find_and_send_request`.
#[cfg(not(test))]
const HIGHER_ID_PATIENCE: Duration = Duration::from_secs(3);
#[cfg(test)]
const HIGHER_ID_PATIENCE: Duration = Duration::from_millis(1_500);

/// `find_and_send_request`'s cap for one dial + hello + ack.
///
/// Against a real phone the dial alone takes 1-2s, and `BleDataLink::send`
/// retries a `GattBusy` rejection for up to ~1.4s on top; 4s was once too
/// short and dropped every attempt mid-connect:
///
/// ```text
/// 02:05:13  connect: dialling 52:E1:52:02:7D:37
/// 02:05:17  connect: abandoned before completing (connect guard dropped)
/// ```
///
/// 15s leaves a 60s setup round room for four candidates -- more than a
/// room ever holds.
const FIND_PEER_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(15);

impl BluetoothChannelService {
    /// One Bluetooth setup search for `peer_id` (ADR-0008 D12): asks the
    /// scanner for the peer for up to `timeout`, and connects the moment it
    /// is heard.
    async fn find_and_connect_plain(&self, peer_id: &str, timeout: Duration) -> Result<Box<dyn DataLink>, String> {
        let scanner = &self.scanner;
        let Some(found) = scanner.find(peer_id, Purpose::Setup, timeout).await else {
            if BluetoothAdapter::this_device().is_unavailable() {
                return Err("bluetooth adapter unavailable".to_string());
            }
            return Err(format!("{peer_id} not heard within {timeout:?}"));
        };
        // Both devices search for each other, and each dials the other the
        // moment it hears it. Two dials crossing fail each other: a device
        // that is itself mid-connect is not connectable, so the other's dial
        // hangs until BlueZ aborts it ("le-connection-abort-by-local" after
        // 8-12 s), and the retry crosses again. Measured on a laptop and a
        // Pixel, this was most of the 40-120 s a setup took. So the device
        // with the higher id lets the other dial first, and dials back once
        // the other's hello has reached it.
        self.wait_for_turn_to_connect(peer_id).await;
        // A candidate probe already dialling is let finish, not cancelled;
        // this dial must not overlap one to the same peer.
        let _probe = scanner.candidate_probe().await;
        // Bounded: a candidate that accepts the connection and never
        // finishes the handshake must not hold the round open.
        let connected = tokio::time::timeout(
            FIND_PEER_CANDIDATE_TIMEOUT,
            self.connect_session(peer_id, &found.address, PEER_ALPN),
        )
        .await
        .map_err(|_| format!("connect to {} timed out", found.address))
        .and_then(|result| result);
        if connected.is_err() {
            scanner.note_connect_failed(&found.address);
        }
        connected
    }

    /// In a Bluetooth setup the device with the lower id dials first; the
    /// one with the higher id waits until the other's hello has reached it
    /// (or `HIGHER_ID_PATIENCE` runs out) before dialling back.
    pub(crate) async fn wait_for_turn_to_connect(&self, peer_id: &str) {
        if self.identity.device_id.as_str() <= peer_id {
            return;
        }
        let patience = tokio::time::Instant::now() + HIGHER_ID_PATIENCE;
        while tokio::time::Instant::now() < patience && !self.peer_request_accepted(peer_id) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// The setup with `peer_id` accepted the peer's `AddChannel`, or the
    /// setup ended (`false`): the device with the higher id connects then.
    pub fn set_peer_request_accepted(&self, peer_id: &str, accepted: bool) {
        if let Ok(mut peers) = self.peer_requests_accepted.lock() {
            if accepted {
                peers.insert(peer_id.to_string());
            } else {
                peers.remove(peer_id);
            }
        }
    }

    fn peer_request_accepted(&self, peer_id: &str) -> bool {
        self.peer_requests_accepted.lock().is_ok_and(|peers| peers.contains(peer_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0008 D12: a device not found is searched for again after 1, 5 and
    /// 15 minutes, then every 15 minutes.
    #[test]
    fn delivery_searches_back_off_one_five_fifteen_then_every_fifteen() {
        let now = std::time::Instant::now();
        let waits: Vec<u64> = (1..=5)
            .map(|misses| next_delivery_search(misses, now).duration_since(now).as_secs())
            .collect();
        assert_eq!(waits, vec![60, 300, 900, 900, 900]);
    }

    /// Switching Bluetooth on tries again at once, whatever retry delay
    /// earlier misses left behind.
    #[tokio::test]
    async fn forgetting_delivery_misses_makes_the_next_search_due_now() {
        let schedule = DeliverySchedule::new(Arc::new(EventBusService::default()));
        let scanner = Scanner::default();
        let peer = "peer-forget-delivery-misses";
        schedule.note_delivery_missed(peer);
        assert!(!schedule.delivery_due(&scanner, peer), "a miss delays the next search");
        schedule.forget_delivery_misses(peer);
        assert!(schedule.delivery_due(&scanner, peer));
    }

    /// The fingerprint must be exactly FNV-1a, because both peers compute it
    /// independently from the same `device_id` and any divergence means they
    /// stop recognising each other -- a failure that would only appear across
    /// an app-version boundary and would look like a radio problem.
    ///
    /// Checked against the published algorithm spelled out here rather than
    /// against a captured constant: this catches a silent swap to a different
    /// hash (`DefaultHasher`, whose output is explicitly unstable across Rust
    /// releases, being the tempting one) without needing a magic number whose
    /// provenance a later reader cannot verify.
    #[test]
    fn fingerprint_is_fnv1a_over_the_device_id() {
        fn reference_fnv1a(input: &str) -> [u8; FINGERPRINT_LEN] {
            let mut hash: u32 = 2_166_136_261;
            for byte in input.as_bytes() {
                hash ^= u32::from(*byte);
                hash = hash.wrapping_mul(16_777_619);
            }
            hash.to_be_bytes()
        }

        for id in ["75700b2e-c970-482f-aa16-63ebdde0a91c", "peer-a", ""] {
            assert_eq!(fingerprint_of(id), reference_fnv1a(id), "mismatch for {id:?}");
        }
        assert_ne!(fingerprint_of("peer-a"), fingerprint_of("peer-b"));
    }

}

mod adapter {
//! The Bluetooth adapter: one per process, and whether it worked the last
//! time anything used it.
//!
//! Process-wide on purpose, unlike the per-state objects beside it: a
//! process drives one physical adapter, and each `ble-gatt` platform driver
//! owns one D-Bus session (Linux) or one Activity-bound plugin (Android).

use super::*;

/// The Bluetooth adapter of this device: the ble-gatt `Adapter`, made one
/// time for the process at the first use; whether it worked the last time
/// anything used it; and, in debug builds, the test switch over its driver.
pub struct BluetoothAdapter {
    adapter: tokio::sync::OnceCell<Adapter>,
    /// See `ADAPTER_UNKNOWN`.
    health: AtomicU8,
    #[cfg(feature = "devtools")]
    switch: std::sync::OnceLock<Arc<test_switch::RadioSwitch>>,
}

static THIS_DEVICE: BluetoothAdapter = BluetoothAdapter {
    adapter: tokio::sync::OnceCell::const_new(),
    health: AtomicU8::new(ADAPTER_UNKNOWN),
    #[cfg(feature = "devtools")]
    switch: std::sync::OnceLock::new(),
};

impl BluetoothAdapter {
    /// The one adapter of this process.
    pub fn this_device() -> &'static Self {
        &THIS_DEVICE
    }

    /// The ble-gatt adapter, made at the first use.
    ///
    /// The Linux driver opens a BlueZ D-Bus session and needs a powered
    /// adapter. Fini makes it at the first use, not at startup, so a
    /// computer without a usable adapter still starts the app.
    #[cfg(target_os = "linux")]
    pub(super) async fn adapter(&self) -> Result<Adapter, String> {
    // `get_or_try_init`, not `get_or_init` over a cached `Result`: the
    // latter permanently bakes in whatever the *first* call returned,
    // success or failure. If Fini starts while Bluetooth is off, BlueZ is
    // restarting, or the adapter is briefly unavailable, that first
    // failure would be replayed to every later dial/serve attempt for the
    // rest of the process's life, even after the adapter comes back —
    // `get_or_try_init` instead leaves the cell empty on error, so the
    // next call genuinely retries construction.
    // Bounded: `LinuxDriver::new()` opens a D-Bus system-bus session and
    // talks to BlueZ over it. On a machine with no `bluetoothd` running at
    // all (headless CI containers, minimal installs) there's nothing wrong
    // locally, but D-Bus's own service-activation/name-owner-wait semantics
    // can leave that call pending far longer than this transport is worth
    // blocking anything on — this runs at app startup via
    // `tauri::async_runtime::spawn`, so an unbounded wait here would tie up
    // an async worker thread for however long that takes. A timeout turns
    // "no adapter reachable" into a fast, retryable failure instead.
        const ADAPTER_INIT_TIMEOUT: Duration = Duration::from_secs(5);

        self.adapter
            .get_or_try_init(|| async {
                #[cfg(feature = "devtools")]
                if let Ok(endpoint) = std::env::var("FINI_BLE_MOCK_BROKER") {
                    return Ok(Adapter::new(self.switched(mock_driver(&endpoint).await?)).await);
                }
                match tokio::time::timeout(ADAPTER_INIT_TIMEOUT, LinuxDriver::new()).await {
                    Ok(Ok(driver)) => Ok(Adapter::new(self.switched(Arc::new(driver))).await),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(_) => Err(format!(
                        "timed out after {ADAPTER_INIT_TIMEOUT:?} waiting for the Bluetooth adapter"
                    )),
                }
            })
            .await
            .cloned()
    }

    /// The ble-gatt adapter on Android, which `tauri-plugin-ble-gatt` makes.
    /// Made at the first use, not at `.setup()`: the driver needs the
    /// Activity, and the Activity does not exist at `.setup()` (see the
    /// plugin's `lazy` module). The first use comes from a command or a tick
    /// after startup (`start_peripheral_once`, `start_exchange`). A
    /// `devtools` build puts the adapter's driver behind the test switch.
    #[cfg(target_os = "android")]
    pub(super) async fn adapter(&self) -> Result<Adapter, String> {
        self.adapter
            .get_or_try_init(|| async {
                let adapter = crate::services::ble_plugin::adapter().await?;
                #[cfg(feature = "devtools")]
                return Ok(Adapter::new(self.switched(adapter.driver())).await);
                #[cfg(not(feature = "devtools"))]
                Ok(adapter)
            })
            .await
            .cloned()
    }

    /// The driver as the adapter gets it. In a build without `devtools` this
    /// is the platform driver itself.
    #[cfg(all(target_os = "linux", not(feature = "devtools")))]
    fn switched(&self, driver: Arc<dyn ble_gatt::Driver>) -> Arc<dyn ble_gatt::Driver> {
        driver
    }

    /// The driver as the adapter gets it. In a `devtools` build the driver is
    /// behind the test switch (`set_switch`).
    #[cfg(feature = "devtools")]
    fn switched(&self, driver: Arc<dyn ble_gatt::Driver>) -> Arc<dyn ble_gatt::Driver> {
        let switch = test_switch::RadioSwitch::new(driver);
        let _ = self.switch.set(switch.clone());
        Arc::new(test_switch::SwitchedDriver(switch))
    }

    /// Sets the test switch of the Bluetooth radio. Off makes the radio look
    /// switched off to Fini: scans and connections fail, links close, and
    /// `RadioChanged` tells ble-gatt. On makes everything pass through
    /// again. The real radio stays on. False if this device has no usable
    /// adapter.
    #[cfg(feature = "devtools")]
    pub async fn set_switch(&self, on: bool) -> bool {
        if self.adapter().await.is_err() {
            return false;
        }
        match self.switch.get() {
            Some(switch) => {
                switch.set(on).await;
                true
            }
            None => false,
        }
    }

    pub(super) fn note_reachable(&self) {
        self.health.store(ADAPTER_REACHABLE, Ordering::Relaxed);
    }

    pub(super) fn note_unreachable(&self) {
        self.health.store(ADAPTER_UNREACHABLE, Ordering::Relaxed);
    }

    /// Records what a refused scan says about the adapter. A busy adapter (a
    /// discovery already running, which ble-gatt has tried to recover) is
    /// there and working -- only something else refuses it as missing.
    pub(super) fn note_scan_refused(&self, err: &ble_gatt::BleError) -> bool {
        if matches!(err, ble_gatt::BleError::AdapterBusy(_)) {
            self.note_reachable();
            true
        } else {
            self.note_unreachable();
            false
        }
    }

    /// `true` only once a genuine attempt has failed -- never merely because
    /// nothing has been tried yet. See `ADAPTER_UNKNOWN`.
    pub fn is_unavailable(&self) -> bool {
        #[cfg(test)]
        if let Some(reachable) = ADAPTER_PINNED.with(|pinned| pinned.get()) {
            return !reachable;
        }
        self.health.load(Ordering::Relaxed) == ADAPTER_UNREACHABLE
    }

    /// Asks the radio, right now, whether it can be used -- and records the
    /// answer into the health like any other attempt.
    ///
    /// The observed signal is free but unhurried: it only refreshes when
    /// the dial loop next scans, which since ADR-0007 is up to 30s away. That
    /// is fine for a row correcting itself in the background and wrong for
    /// the one moment the user is watching -- switching a channel on and
    /// expecting to be told immediately if their own Bluetooth is off. So
    /// this exists to be called from that user action, and only from it.
    ///
    /// It opens a discovery session and drops it immediately rather than
    /// asking BlueZ whether the adapter reports itself powered. An adapter
    /// that claims to be powered and then refuses to scan is a real failure
    /// mode, and the question worth answering is "can we do the thing", not
    /// "does the hardware feel well".
    ///
    /// A scan already running elsewhere answers the question without a
    /// second one: the adapter accepted that discovery session, so it is
    /// reachable. Starting our own would only be refused (see `scan_lock`).
    /// A caller that merely holds the lease proves nothing yet -- its scan
    /// may still be refused -- so the probe waits for it to start or finish.
    pub async fn probe(&self, scanner: &Scanner) -> bool {
        let deadline = tokio::time::Instant::now() + PROBE_WAIT_FOR_OTHER_SCAN;
        loop {
            if scanner.scan_is_running() {
                self.note_reachable();
                return true;
            }
            if let Some(_scan) = scanner.try_scan_lock() {
                let Ok(adapter) = self.adapter().await else {
                    self.note_unreachable();
                    return false;
                };
                return match adapter.central().scan(datagram_config().service).await {
                    Ok(stream) => {
                        drop(stream);
                        self.note_reachable();
                        true
                    }
                    Err(err) => {
                        log::warn!("[transport][ble] adapter probe failed: {err}");
                        self.note_scan_refused(&err)
                    }
                };
            }
            // Another caller holds the lease but its scan has not started:
            // wait for it to start (the adapter works) or give the lease up.
            if tokio::time::Instant::now() >= deadline {
                return !self.is_unavailable();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// For a test on this thread that is not about the adapter: the
    /// process-wide health is written by concurrent tests (and by any scan
    /// they start), so a check that reads it would pass or fail with their
    /// timing.
    #[cfg(test)]
    pub fn pin_reachable_on_this_thread() {
        ADAPTER_PINNED.with(|pinned| pinned.set(Some(true)));
    }

    /// For a test about what an unreachable adapter does, for the same
    /// reason.
    #[cfg(test)]
    fn pin_unreachable_on_this_thread() {
        ADAPTER_PINNED.with(|pinned| pinned.set(Some(false)));
    }
}

/// The `actors-ble` e2e lane's radio: a `MockDriver` talking to a
/// cross-process broker instead of BlueZ, so two real `fini-app` processes
/// can prove this file's dial/peripheral/session-claim code path without
/// hardware. See `specs/e2e/actors/helpers/ble-sync.ts` and ble-gatt's
/// `docs/adr/0004-mock-broker-for-cross-process-e2e.md`.
///
/// Both the `devtools` feature gate above (off in release builds) and this
/// env var must be true for a process to ever reach this branch -- an env
/// var alone can never redirect a shipped binary onto a socket.
#[cfg(all(target_os = "linux", feature = "devtools"))]
async fn mock_driver(endpoint: &str) -> Result<Arc<dyn ble_gatt::Driver>, String> {
    use ble_gatt::drivers::mock::{MockDriver, MockNetwork};
    use ble_gatt::CapabilityReport;

    // Reuses the existing "fake local adapter address" escape hatch
    // (`pairing::commands::local_bluetooth_address`'s own env
    // override) rather than adding a second one -- the harness already sets
    // this per actor for the self-report path.
    let address = std::env::var("FINI_LOCAL_BLUETOOTH_ADDRESS")
        .map_err(|_| "FINI_BLE_MOCK_BROKER is set but FINI_LOCAL_BLUETOOTH_ADDRESS is not".to_string())?;
    let network = MockNetwork::remote(endpoint)
        .await
        .map_err(|err| format!("ble mock broker connect to {endpoint} failed: {err}"))?;
    Ok(Arc::new(MockDriver::new(
        PeerAddress(address),
        network,
        CapabilityReport { central: true, peripheral: true },
    )) as Arc<dyn ble_gatt::Driver>)
}

/// A test switch over the Bluetooth driver, for debug builds only. It
/// makes the radio look off without a change to the real radio, so that
/// no test must switch off the Bluetooth of a person's device.
#[cfg(feature = "devtools")]
mod test_switch {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ble_gatt::{
        BleError, BoxStream, Driver, CapabilityReport, CharacteristicUuid, ConnectionPriority,
        DiscoveredPeer, GattConnection, GattEvent, GattServiceSpec, PeerAddress, RadioStatus, Role,
        ServiceUuid, WriteType,
    };
    use futures_util::StreamExt;
    use tokio::sync::broadcast;

    fn switched_off() -> BleError {
        BleError::AdapterUnavailable("the test switch set the radio to off".into())
    }

    pub(super) struct RadioSwitch {
        driver: Arc<dyn Driver>,
        off: AtomicBool,
        /// The events the switch adds: `RadioChanged`, and `Disconnected`
        /// for each link that it closes.
        events: broadcast::Sender<GattEvent>,
        /// The open links, from the driver's own events.
        links: Mutex<Vec<(PeerAddress, Role, Option<u64>)>>,
    }

    impl RadioSwitch {
        pub(super) fn new(driver: Arc<dyn Driver>) -> Arc<Self> {
            let switch = Arc::new(Self {
                driver: driver.clone(),
                off: AtomicBool::new(false),
                events: broadcast::channel(64).0,
                links: Mutex::new(Vec::new()),
            });
            let weak = Arc::downgrade(&switch);
            let mut events = driver.events();
            tokio::spawn(async move {
                while let Some(event) = events.next().await {
                    let Some(switch) = weak.upgrade() else { return };
                    switch.record(&event);
                }
            });
            switch
        }

        fn is_off(&self) -> bool {
            self.off.load(Ordering::SeqCst)
        }

        fn record(&self, event: &GattEvent) {
            let mut links = self.links.lock().unwrap();
            match event {
                GattEvent::Connected { peer, local_role, session } => {
                    links.push((peer.clone(), *local_role, *session));
                }
                GattEvent::Disconnected { peer, local_role, session } => {
                    links.retain(|(p, r, s)| !(p.0 == peer.0 && r == local_role && s == session));
                }
                _ => {}
            }
        }

        pub(super) async fn set(&self, on: bool) {
            if on {
                if self.off.swap(false, Ordering::SeqCst) {
                    log::info!("[transport][ble] test switch: radio on");
                    let _ = self.events.send(GattEvent::RadioChanged { status: RadioStatus::On });
                }
                return;
            }
            if self.off.swap(true, Ordering::SeqCst) {
                return;
            }
            log::info!("[transport][ble] test switch: radio off");
            let _ = self.driver.stop_advertising().await;
            let links: Vec<_> = std::mem::take(&mut *self.links.lock().unwrap());
            for (peer, local_role, session) in links {
                if local_role == Role::Peripheral {
                    let _ = self.driver.disconnect_peer(&peer, session).await;
                }
                let _ = self.events.send(GattEvent::Disconnected { peer, local_role, session });
            }
            let _ = self.events.send(GattEvent::RadioChanged { status: RadioStatus::Off });
        }
    }

    pub(super) struct SwitchedDriver(pub(super) Arc<RadioSwitch>);

    #[async_trait]
    impl Driver for SwitchedDriver {
        async fn capabilities(&self) -> CapabilityReport {
            self.0.driver.capabilities().await
        }

        async fn scan(&self, service: ServiceUuid) -> ble_gatt::Result<BoxStream<ble_gatt::Result<DiscoveredPeer>>> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            let switch = self.0.clone();
            let stream = self.0.driver.scan(service).await?;
            Ok(Box::pin(stream.map(move |item| if switch.is_off() { Err(switched_off()) } else { item })))
        }

        async fn connect(&self, peer: &PeerAddress) -> ble_gatt::Result<Box<dyn GattConnection>> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            let inner = self.0.driver.connect(peer).await?;
            Ok(Box::new(SwitchedConnection { inner, switch: self.0.clone() }))
        }

        async fn advertise(&self, service: GattServiceSpec) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.advertise(service).await
        }

        async fn update_advertisement(&self, service: GattServiceSpec) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.update_advertisement(service).await
        }

        async fn stop_advertising(&self) -> ble_gatt::Result<()> {
            self.0.driver.stop_advertising().await
        }

        async fn notify(&self, characteristic: CharacteristicUuid, value: Vec<u8>) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.notify(characteristic, value).await
        }

        async fn notify_peer(
            &self, peer: &PeerAddress, session: Option<u64>, characteristic: CharacteristicUuid, value: Vec<u8>,
        ) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.notify_peer(peer, session, characteristic, value).await
        }

        async fn disconnect_peer(&self, peer: &PeerAddress, session: Option<u64>) -> ble_gatt::Result<()> {
            self.0.driver.disconnect_peer(peer, session).await
        }

        async fn notify_max_len(&self, peer: &PeerAddress, session: Option<u64>) -> Option<usize> {
            self.0.driver.notify_max_len(peer, session).await
        }

        fn events(&self) -> BoxStream<GattEvent> {
            // While the switch is off, writes from centrals do not arrive,
            // as with a radio that is off.
            let switch = self.0.clone();
            let from_driver = self.0.driver.events().filter(move |event| {
                let drop_it = switch.is_off() && matches!(event, GattEvent::CharacteristicWritten { .. });
                std::future::ready(!drop_it)
            });
            let from_switch = futures_util::stream::unfold(self.0.events.subscribe(), |mut rx| async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => return Some((event, rx)),
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            });
            Box::pin(futures_util::stream::select(from_driver, from_switch))
        }

        async fn radio_status(&self) -> RadioStatus {
            if self.0.is_off() {
                return RadioStatus::Off;
            }
            self.0.driver.radio_status().await
        }
    }

    /// A central-role connection that fails while the switch is off.
    struct SwitchedConnection {
        inner: Box<dyn GattConnection>,
        switch: Arc<RadioSwitch>,
    }

    #[async_trait]
    impl GattConnection for SwitchedConnection {
        fn peer(&self) -> PeerAddress {
            self.inner.peer()
        }

        fn session(&self) -> Option<u64> {
            self.inner.session()
        }

        fn att_mtu(&self) -> u16 {
            self.inner.att_mtu()
        }

        async fn read(&mut self, characteristic: CharacteristicUuid) -> ble_gatt::Result<Vec<u8>> {
            if self.switch.is_off() {
                return Err(switched_off());
            }
            self.inner.read(characteristic).await
        }

        async fn write_with_type(
            &mut self, characteristic: CharacteristicUuid, value: Vec<u8>, write_type: WriteType,
        ) -> ble_gatt::Result<()> {
            if self.switch.is_off() {
                return Err(switched_off());
            }
            self.inner.write_with_type(characteristic, value, write_type).await
        }

        async fn subscribe(
            &mut self, characteristic: CharacteristicUuid,
        ) -> ble_gatt::Result<BoxStream<ble_gatt::Result<Vec<u8>>>> {
            if self.switch.is_off() {
                return Err(switched_off());
            }
            let switch = self.switch.clone();
            let stream = self.inner.subscribe(characteristic).await?;
            Ok(Box::pin(stream.take_while(move |_| std::future::ready(!switch.is_off()))))
        }

        async fn disconnect(&mut self) -> ble_gatt::Result<()> {
            self.inner.disconnect().await
        }

        async fn request_connection_priority(&mut self, priority: ConnectionPriority) -> ble_gatt::Result<()> {
            self.inner.request_connection_priority(priority).await
        }
    }
}

/// Whether the *local* Bluetooth radio could actually be used the last time
/// this process tried to use it.
///
/// Three-valued on purpose. Before anything has been attempted the honest
/// answer is "nobody has looked", and a row must not tell the user their
/// adapter is off on that basis -- the same reasoning that makes
/// `bluetooth_peer_nearby_now` default to `true` on a platform that cannot
/// look.
///
/// Deliberately *observed* rather than polled. Asking BlueZ whether the
/// adapter is powered costs a D-Bus round trip, and the transport row is
/// polled every few seconds for every paired peer; the dial loop already
/// searches whenever it has a reason to, so recording what those attempts find
/// keeps this fresh for free. It is also the truer signal: "we asked the
/// radio to do something and it refused" is what the user actually cares
/// about, and it catches an adapter that reports itself powered while
/// refusing to scan.
///
/// Only a real *use* records health. Recording success from `adapter()`
/// alone would be wrong: it caches the adapter in a `OnceCell`, so once the
/// adapter has worked once, every later call returns that cached handle
/// without touching hardware -- an adapter switched off afterwards would
/// still look reachable, and the row would flap between the cached success
/// and the scan failure that follows it.
const ADAPTER_UNKNOWN: u8 = 0;
const ADAPTER_REACHABLE: u8 = 1;
const ADAPTER_UNREACHABLE: u8 = 2;

#[cfg(test)]
thread_local! {
    // The adapter health a test pinned on its thread, if any.
    static ADAPTER_PINNED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// How long the probe waits on another caller's scan that is still being
/// set up, before falling back to the last recorded adapter health.
const PROBE_WAIT_FOR_OTHER_SCAN: Duration = Duration::from_secs(10);


#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that set `FINI_LOCAL_BLUETOOTH_ADDRESS`/
    /// `FINI_BLE_MOCK_BROKER` -- process-global env vars, unsafe to mutate
    /// from parallel test threads. Mirrors `BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK`
    /// in `pairing::commands`.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    static MOCK_DRIVER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Proves `mock_driver` actually round-trips through a real socket to a
    /// real `MockNetwork::serve` broker, not just that it compiles -- the
    /// same connect+advertise+scan shape `ble-gatt`'s own
    /// `tests/mock_broker.rs` exercises, but through fini's injection point
    /// (`FINI_BLE_MOCK_BROKER`/`FINI_LOCAL_BLUETOOTH_ADDRESS`) instead of
    /// calling `MockNetwork::remote` directly. This is what the `actors-ble`
    /// e2e lane's two real `fini-app` processes will each do at startup.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    #[tokio::test]
    async fn mock_driver_connects_to_a_broker_and_reports_full_capabilities() {
        let _guard = MOCK_DRIVER_ENV_LOCK.lock().unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback broker port");
        let broker_addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = ble_gatt::drivers::mock::MockNetwork::serve(listener).await;
        });

        std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "AA:BB:CC:00:00:01");
        let result = mock_driver(&broker_addr.to_string()).await;
        std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");

        let driver = result.expect("mock_driver should connect to the broker");
        let capabilities = driver.capabilities().await;
        assert!(capabilities.central && capabilities.peripheral);
    }

    /// The one precondition `mock_driver` itself enforces: without a local
    /// address there is nothing to identify this peer as on the mock radio,
    /// and the error must name which env var is missing rather than panic
    /// or silently pick something.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    #[tokio::test]
    async fn mock_driver_requires_a_local_bluetooth_address() {
        let _guard = MOCK_DRIVER_ENV_LOCK.lock().unwrap();
        std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");

        let err = match mock_driver("127.0.0.1:1").await {
            Ok(_) => panic!("must fail without a local address"),
            Err(err) => err,
        };
        assert!(err.contains("FINI_LOCAL_BLUETOOTH_ADDRESS"));
    }

    /// ble-gatt reports a discovery it could not take over as busy, not
    /// missing: that adapter works, so Bluetooth must not read as unavailable.
    #[test]
    fn a_busy_adapter_is_not_an_unavailable_one() {
        let adapter = BluetoothAdapter::this_device();
        assert!(adapter.note_scan_refused(&ble_gatt::BleError::AdapterBusy("in progress".into())));
        assert!(!adapter.note_scan_refused(&ble_gatt::BleError::AdapterUnavailable("no adapter".into())));
    }

    /// A scan already running is proof the adapter works, so the check a
    /// person triggers by switching Bluetooth on must say so -- not start a
    /// second scan, which Android refuses ("a scan is already active") and
    /// which used to be recorded as "Bluetooth is unavailable on this device".
    #[tokio::test]
    async fn adapter_probe_during_another_scan_reports_the_adapter_reachable() {
        let radio = Scanner::default();
        let _other_scan = radio.scan_lock().await;
        let _running = radio.start_running_scan();
        let adapter = BluetoothAdapter::this_device();
        adapter.note_unreachable();

        assert!(
            adapter.probe(&radio).await,
            "a scan in flight means the adapter accepted a discovery session"
        );
        assert!(!adapter.is_unavailable());
    }

    /// Holding the lease is not a running scan. The add-mode scan takes the
    /// lease and then waits on the adapter; a probe in that window used to
    /// report a missing radio as working, so adding Bluetooth never failed.
    #[tokio::test(start_paused = true)]
    async fn adapter_probe_does_not_trust_a_scan_that_has_not_started() {
        let radio = Arc::new(Scanner::default());
        let other_scan = radio.scan_lock().await;
        // Pinned, not recorded: other tests' scans write the process-wide
        // health while this one waits out the probe.
        BluetoothAdapter::pin_unreachable_on_this_thread();

        let probe = tokio::spawn({
            let radio = radio.clone();
            async move { BluetoothAdapter::this_device().probe(&radio).await }
        });
        tokio::time::sleep(PROBE_WAIT_FOR_OTHER_SCAN + Duration::from_secs(1)).await;
        drop(other_scan);

        assert!(!probe.await.unwrap(), "no scan ever started, and the adapter was last seen unreachable");
    }
}
}

mod advertiser {
//! What this device advertises over Bluetooth, and whether it advertises
//! at all.
//!
//! One `Advertiser` per Bluetooth channel, so two devices in one test
//! process each advertise their own identity and add-mode.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use std::time::Instant;

use tokio::sync::watch;

use super::{datagram_config, fingerprint_of, ADD_MODE_FLAG_BYTE, FINGERPRINT_LEN, FINI_MANUFACTURER_ID};
use ble_gatt::datagram::DatagramConfig;
use crate::models::Identity;

/// How long this device stays reachable after a Bluetooth init completes
/// here. Its accept of the peer's `AddChannel` can be lost when the link
/// drops, so the peer may still search; the accept is still given
/// (ADR-0008 D2), but only if the peer can find this device.
pub(super) const ANSWER_AFTER_SETUP: Duration = Duration::from_secs(120);

/// The longest name the scan response carries on every platform: Android
/// fits 13 bytes next to the service UUID, Linux 29. Cut here, so both show
/// the same name.
const SHORT_NAME_MAX_BYTES: usize = 13;

/// `name` cut to `SHORT_NAME_MAX_BYTES` at a character boundary.
fn short_name(name: &str) -> String {
    let mut end = name.len().min(SHORT_NAME_MAX_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

pub struct Advertiser {
    /// This device's advertised identity fingerprint: what lets a scanning
    /// peer tell this device apart from any other Fini install without
    /// connecting to it first.
    fingerprint: [u8; FINGERPRINT_LEN],
    /// This device's name, cut to `SHORT_NAME_MAX_BYTES`, sent in the scan
    /// response so the Add Device list can show it without a connection.
    short_name: String,
    /// Whether this device is in add-mode, watched by `run_server`'s
    /// peripheral loop so a toggle can trigger a fresh advertisement
    /// carrying (or dropping) the add-mode flag without restarting the whole
    /// peripheral task -- Android's `start_peripheral_once` is deliberately a
    /// one-time start (ADR-0002 in ble-gatt: constructing `AndroidDriver`
    /// outside a genuine post-startup call panics), so the *outer* task can
    /// never be torn down and re-spawned to pick up a config change; only the
    /// inner advertise/accept loop can be.
    ///
    /// `watch::Sender` alone is enough: it has its own `borrow()` for a
    /// snapshot read (`advertised_config`), and `subscribe()` hands out a
    /// fresh `Receiver` for whichever caller needs to *wait* on a change
    /// (`run_server`, in a `tokio::select!` against the incoming-connections
    /// stream).
    add_mode: watch::Sender<bool>,
    /// Whether this device should advertise (ADR-0008 D8): while it has a
    /// Bluetooth channel on, is setting one up, or is being paired. Otherwise
    /// the radio stays quiet -- there is nobody it should be reachable by.
    wanted: watch::Sender<bool>,
    answer_after_setup_until: StdMutex<Option<Instant>>,
    /// One peripheral acceptor, enforced where the loop actually runs rather
    /// than at each call site.
    ///
    /// This used to be a `Once` inside `start_peripheral_once`, which guarded
    /// only that one caller. `start_serving` spawned `run_server` directly
    /// as well, so on Android both ran: two accept loops, both handed the
    /// same inbound central, both running the gate on it. One claimed the
    /// session, the other was rejected as a duplicate, and dropping the
    /// rejected link released the session the winner was using.
    peripheral_running: AtomicBool,
}

impl Advertiser {
    pub fn new(identity: &Identity) -> Self {
        Self {
            fingerprint: fingerprint_of(&identity.device_id),
            short_name: short_name(&identity.hostname),
            add_mode: watch::channel(false).0,
            wanted: watch::channel(false).0,
            answer_after_setup_until: StdMutex::new(None),
            peripheral_running: AtomicBool::new(false),
        }
    }

    /// The datagram config `run_server` advertises: Fini's service plus one
    /// flags byte and the identity fingerprint as manufacturer data, and the
    /// short name in the scan response.
    pub fn advertised_config(&self) -> DatagramConfig {
        let mut config = datagram_config();
        let mut payload = Vec::with_capacity(1 + FINGERPRINT_LEN);
        payload.push(if self.in_add_mode() { ADD_MODE_FLAG_BYTE } else { 0 });
        payload.extend_from_slice(&self.fingerprint);
        config.advertised_manufacturer_data.insert(FINI_MANUFACTURER_ID, payload);
        config.advertised_local_name = Some(self.short_name.clone());
        config
    }

    pub fn in_add_mode(&self) -> bool {
        *self.add_mode.borrow()
    }

    /// Called from `device_connection_enter_add_mode`/`leave_add_mode`; see
    /// `add_mode` for why this signals a running `run_server` rather than
    /// restarting it.
    pub fn set_add_mode(&self, enabled: bool) {
        self.add_mode.send_if_modified(|current| {
            if *current == enabled {
                return false;
            }
            *current = enabled;
            true
        });
    }

    pub fn watch_add_mode(&self) -> watch::Receiver<bool> {
        self.add_mode.subscribe()
    }

    pub fn watch_wanted(&self) -> watch::Receiver<bool> {
        self.wanted.subscribe()
    }

    /// Records whether to advertise; see `refresh_advertising`.
    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.send_if_modified(|current| {
            let changed = *current != wanted;
            *current = wanted;
            changed
        });
    }

    pub fn answering_after_setup(&self) -> bool {
        self.answer_after_setup_until
            .lock()
            .ok()
            .and_then(|until| *until)
            .is_some_and(|until| Instant::now() < until)
    }

    /// A Bluetooth init completed here: keep advertising for
    /// `ANSWER_AFTER_SETUP`.
    pub fn keep_answering_after_setup(&self) {
        if let Ok(mut until) = self.answer_after_setup_until.lock() {
            *until = Some(Instant::now() + ANSWER_AFTER_SETUP);
        }
    }

    /// Takes the peripheral role; false if an acceptor already runs.
    pub fn claim_peripheral_role(&self) -> bool {
        !self.peripheral_running.swap(true, Ordering::SeqCst)
    }

    #[cfg(target_os = "android")]
    pub fn peripheral_running(&self) -> bool {
        self.peripheral_running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(device_id: &str) -> Identity {
        let secret_key = iroh::SecretKey::generate();
        Identity {
            device_id: device_id.to_string(),
            hostname: "host".to_string(),
            endpoint_id: secret_key.public().to_string(),
            secret_key,
        }
    }

    /// The peripheral acceptor must be a singleton, whatever starts it.
    ///
    /// It was not: `start_serving` spawned `run_server` on Linux *and*
    /// Android, while Android also started it from `dial` via
    /// `start_peripheral_once`. Two accept loops then received the same
    /// inbound central and both ran the gate on it -- one claimed the
    /// session, the other was rejected as a duplicate, and dropping the
    /// rejected link released the session the winner was using. Observed on
    /// hardware as an inbound link dying ~170ms after authenticating, with
    /// `no live notify session` as the only trace.
    #[test]
    fn only_one_peripheral_acceptor_can_run_at_a_time() {
        let advertiser = Advertiser::new(&identity("test-device"));
        assert!(advertiser.claim_peripheral_role(), "the first start takes the role");
        assert!(
            !advertiser.claim_peripheral_role(),
            "a second start must be refused -- two acceptors race each other's session"
        );
    }

    #[test]
    fn the_advertisement_carries_the_add_mode_flag_only_while_enabled() {
        // ADR-0006 slice 2 changed the shape this asserts. The payload is no
        // longer present only in add-mode and no longer equals a single
        // flag byte: it is a flags byte followed by the identity
        // fingerprint, advertised always, because being identifiable is what
        // lets a scanner skip peers it does not want. The add-mode signal
        // became bit 0 of that first byte.
        let advertiser = Advertiser::new(&identity("test-device"));
        let expected = fingerprint_of("test-device");

        let disabled = advertiser.advertised_config();
        let payload = disabled
            .advertised_manufacturer_data
            .get(&FINI_MANUFACTURER_ID)
            .expect("the fingerprint is advertised regardless of add-mode");
        assert_eq!(payload[0] & ADD_MODE_FLAG_BYTE, 0, "add-mode bit must be clear");
        assert_eq!(&payload[1..], &expected, "fingerprint must be advertised");

        advertiser.set_add_mode(true);
        let enabled = advertiser.advertised_config();
        let payload = enabled
            .advertised_manufacturer_data
            .get(&FINI_MANUFACTURER_ID)
            .expect("payload present in add-mode too");
        assert_eq!(
            payload[0] & ADD_MODE_FLAG_BYTE,
            ADD_MODE_FLAG_BYTE,
            "add-mode bit must be set while add-mode is on"
        );
        assert_eq!(&payload[1..], &expected, "fingerprint is unchanged by add-mode");

        advertiser.set_add_mode(false);
        let disabled_again = advertiser.advertised_config();
        assert_eq!(
            disabled_again.advertised_manufacturer_data.get(&FINI_MANUFACTURER_ID).map(|p| p[0]
                & ADD_MODE_FLAG_BYTE),
            Some(0),
            "must stop signalling add-mode once it is left again"
        );
    }

    /// After a Bluetooth init completes here the device stays reachable a
    /// while, so a peer that missed its ack can still find it and ask again.
    #[test]
    fn a_finished_bluetooth_setup_keeps_advertising_a_while() {
        let advertiser = Advertiser::new(&identity("test-device"));
        advertiser.keep_answering_after_setup();
        assert!(advertiser.answering_after_setup());
    }
}
}

mod data_link {
//! A plain BLE GATT link, for pre-pairing frames only: there is no pinned
//! key to dial yet, so each frame carries the sender's key itself.

use super::*;

pub struct BleDataLink {
    channel: DatagramChannel,
    peer_addr: String,
    /// A datagram already read off the channel (to route it, see
    /// `route_inbound`), returned by the first `recv`.
    pending: Option<Vec<u8>>,
}

impl BleDataLink {
    pub(super) fn new(channel: DatagramChannel) -> Self {
        let peer_addr = channel.peer().0.clone();
        Self {
            channel,
            peer_addr,
            pending: None,
        }
    }

    pub(super) fn after(channel: DatagramChannel, first: Vec<u8>) -> Self {
        Self {
            pending: Some(first),
            ..Self::new(channel)
        }
    }
}

#[async_trait]
impl DataLink for BleDataLink {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn peer_addr(&self) -> Option<String> {
        Some(self.peer_addr.clone())
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        // Retry `GattBusy` here rather than inside `ble-gatt`: the backend
        // makes exactly one attempt and classifies a rejection it believes is
        // transient as `BleError::GattBusy`, deliberately leaving the retry
        // budget to whoever knows the caller's own deadline. No single
        // backend-side budget can be right for every caller -- a chain sized
        // for a generous one silently exceeds a tight one and reads as a
        // caller-abandoned future rather than an honest failure.
        //
        // Sized to the tightest caller on this path: `scan_add_mode_candidates`
        // gets `BLUETOOTH_SCAN_DURATION_MS` (4s) for the *whole* pass, dial
        // included, and `BluetoothChannel::find_and_send_request` allows `FIND_PEER_CANDIDATE_TIMEOUT`
        // (4s) per candidate. With a dial typically eating 1-2s of that, the
        // ~1.4s worst case below still leaves the caller room to fail cleanly
        // instead of being cut off mid-retry.
        //
        // Observed on real hardware: the *first* write on a freshly connected
        // channel is the one that gets rejected (msg_id=0, fragment 0), while
        // the `subscribe` moments earlier on the same link succeeds -- so this
        // covers a genuine just-connected window, not a dead peer.
        const SEND_RETRY_DELAYS: [Duration; 3] = [
            Duration::from_millis(150),
            Duration::from_millis(300),
            Duration::from_millis(600),
        ];

        let mut attempt = 0;
        loop {
            match self.channel.send(payload.clone()).await {
                Ok(()) => return Ok(()),
                Err(ble_gatt::BleError::GattBusy(err)) if attempt < SEND_RETRY_DELAYS.len() => {
                    log::warn!(
                        "[transport][ble] send to {} rejected as busy ({err}), retrying ({}/{})",
                        self.peer_addr,
                        attempt + 1,
                        SEND_RETRY_DELAYS.len()
                    );
                    tokio::time::sleep(SEND_RETRY_DELAYS[attempt]).await;
                    attempt += 1;
                }
                Err(err) => return Err(err.to_string()),
            }
        }
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        if let Some(first) = self.pending.take() {
            return Some(Ok(first));
        }
        match self.channel.recv().await? {
            Ok(bytes) => Some(Ok(bytes)),
            Err(err) => Some(Err(err.to_string())),
        }
    }
}

/// Central role: connect to a peer's Bluetooth address over a plain link,
/// for Fini's own pre-pairing frames (identify, pair request/accept/
/// complete). Sessions use `BluetoothChannel::connect_session`.
pub async fn connect(address: &str) -> Result<Box<dyn DataLink>, String> {
    let adapter = BluetoothAdapter::this_device().adapter().await?;
    let peer = PeerAddress(address.to_string());
    let channel = datagram::connect(&adapter, &peer, &datagram_config())
        .await
        .map_err(|err| format!("ble connect to {address} failed: {err}"))?;
    Ok(Box::new(BleDataLink::new(channel)))
}
}

mod endpoint {
//! The Bluetooth channel's iroh endpoint (ADR-0009 D2, D3): dialling a
//! session or a pairing step by key, and handing what peers open to the gate.

use super::*;

/// How long a pairing dial keeps retrying a refusal caused by the candidate
/// scan's cancelled probe still tearing down its connection.
const PAIRING_CONNECT_RETRY_WINDOW: Duration = Duration::from_secs(5);

/// How long a pairing dial may take once it has the radio: its retries, one
/// connect that runs to `SESSION_CONNECT_TIMEOUT`, and opening the stream.
/// Counted from after any candidate probe it waited out, so a probe that
/// held the radio does not eat the dial's own time.
const PAIRING_CONNECT_BUDGET: Duration =
    Duration::from_secs(PAIRING_CONNECT_RETRY_WINDOW.as_secs() + SESSION_CONNECT_TIMEOUT.as_secs() + 5);

/// How long a new inbound channel may take to send its first datagram,
/// which decides what it carries (`route_inbound`).
const FIRST_DATAGRAM_TIMEOUT: Duration = Duration::from_secs(15);

/// How long an iroh connection over Bluetooth may take to open. A GATT
/// connect alone was measured at up to ~28 s on hardware.
const SESSION_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a BLE link stays after Fini closed the iroh connection on it, so
/// that the close and its code reach the peer first (`close_links_to`).
pub(crate) const TIME_BEFORE_LINK_CLOSE: Duration = Duration::from_secs(1);

/// How long a pairing's BLE link waits for the first session to use it
/// (`keep_pairing_link`).
const PAIRING_LINK_KEEP_TIME: Duration = Duration::from_secs(15);

impl BluetoothChannelService {
    /// Opens an iroh connection to `peer_key` for a pairing step --
    /// `address` is only where to reach it first (ADR-0009 D2/D7: Bluetooth
    /// carries iroh, and iroh dials by key) -- and hands back the link plus
    /// the registrations that keep it clear of the scan: no scan runs until
    /// the caller drops the returned guards.
    ///
    /// Waits out a candidate probe already dialling, then dials within
    /// `PAIRING_CONNECT_BUDGET`.
    ///
    /// The peer's key is not pinned yet, so TLS proves only that whoever
    /// answers holds `peer_key`; pairing pins it (D8).
    pub async fn connect_for_pairing(&self, peer_key: &str, address: &str) -> Result<(Box<dyn DataLink>, impl Sized), String> {
        let peer_key: iroh::EndpointId = peer_key
            .parse()
            .map_err(|err| format!("invalid key for the pairing peer: {err}"))?;
        let scanner = &self.scanner;
        let step = scanner.begin_pairing_step();
        // A candidate probe already dialling is let finish, not cancelled;
        // wait for it so this dial never overlaps one to the same peer.
        drop(scanner.candidate_probe().await);
        tokio::time::timeout(PAIRING_CONNECT_BUDGET, async {
            let connect_guard = scanner.acquire_connect().await;
            let (endpoint, transport) = self.iroh_endpoint().await?;
            let peer = PeerAddress(address.to_string());
            transport.set_peer_address(peer_key, peer.clone());
            let addr = iroh::EndpointAddr::from_parts(
                peer_key,
                [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(&peer))],
            );
            let started = tokio::time::Instant::now();
            loop {
                let attempt =
                    tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr.clone(), PEER_ALPN)).await;
                match attempt {
                    Ok(Ok(connection)) => {
                        let link =
                            IrohDataLink::open(ChannelKind::Bluetooth, connection, Some(address.to_string())).await?;
                        return Ok((Box::new(link) as Box<dyn DataLink>, connect_guard));
                    }
                    // The probe's connection may still be closing.
                    Ok(Err(_)) if started.elapsed() < PAIRING_CONNECT_RETRY_WINDOW => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    Ok(Err(err)) => return Err(format!("bluetooth connect to {address} failed: {err}")),
                    Err(_) => return Err(format!("bluetooth connect to {address} timed out")),
                }
            }
        })
        .await
        .map_err(|_| format!("bluetooth pairing connection to {address} timed out"))?
        .map(|(link, connect_guard)| (link, (step, connect_guard)))
    }

    /// The channel's iroh endpoint over Bluetooth (ADR-0009 D2, D3): the
    /// `ble-gatt-iroh` transport only, no IP, relays off. Built on first
    /// use, which on Android has to come after the Activity is up
    /// (`BluetoothAdapter::adapter`). Its `Router` hands every connection to
    /// the handler of its ALPN.
    pub(crate) async fn iroh_endpoint(&self) -> Result<(iroh::Endpoint, ble_gatt_iroh::BleGattTransport), String> {
        self.endpoint
            .get_or_try_init(|| async {
                let adapter = BluetoothAdapter::this_device().adapter().await?;
                let transport = ble_gatt_iroh::BleGattTransport::builder()
                    .dialer(ble_gatt_iroh::dial_with(adapter, datagram_config()))
                    .build();
                let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .secret_key(self.identity.secret_key.clone())
                    .relay_mode(iroh::RelayMode::Disabled)
                    .alpns(vec![PEER_ALPN.to_vec(), SYNC_ALPN.to_vec()])
                    .transport_config(crate::services::channel_service::iroh_link::transport_config())
                    .clear_ip_transports()
                    .add_custom_transport(Arc::new(transport.clone()))
                    .address_lookup(transport.address_lookup())
                    .bind()
                    .await
                    .map_err(|err| format!("binding the bluetooth endpoint failed: {err}"))?;
                match self.handlers.get().cloned() {
                    Some(handlers) => {
                        tokio::spawn(crate::services::channel_service::LinkAcceptor::serve(
                            ChannelKind::Bluetooth,
                            endpoint.clone(),
                            handlers,
                            Arc::new(|| false),
                        ));
                    }
                    None => log::warn!("[transport][ble] no link handlers; the endpoint answers nobody"),
                }
                Ok::<_, String>((endpoint, transport))
            })
            .await
            .cloned()
    }

    /// Opens an authenticated link to paired `peer_id` at `address`: an iroh
    /// connection over Bluetooth to the key pinned for the pair, which TLS
    /// proves (ADR-0009 D8). A device at that address holding another key
    /// fails the handshake, so this also confirms the candidate is the peer.
    /// `alpn`: `SYNC_ALPN` for a session, `PEER_ALPN` for `AddChannel`.
    pub async fn connect_session(&self, peer_id: &str, address: &str, alpn: &[u8]) -> Result<Box<dyn DataLink>, String> {
        let peer_key: iroh::EndpointId = self
            .pinned_key(peer_id)
            .ok_or_else(|| format!("{peer_id} has no key pinned; pair the devices again"))?
            .parse()
            .map_err(|err| format!("invalid key pinned for {peer_id}: {err}"))?;
        let (endpoint, transport) = self.iroh_endpoint().await?;
        let peer = PeerAddress(address.to_string());
        transport.set_peer_address(peer_key, peer.clone());
        let addr = iroh::EndpointAddr::from_parts(
            peer_key,
            [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(&peer))],
        );
        let connection = tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr, alpn))
            .await
            .map_err(|_| format!("bluetooth connect to {address} timed out"))?
            .map_err(|err| format!("bluetooth connect to {address} failed: {err}"))?;
        let link = IrohDataLink::open(ChannelKind::Bluetooth, connection, Some(address.to_string())).await?;
        Ok(Box::new(link))
    }

    /// After `after`, closes the BLE links to `peer_key`: at its current
    /// address and at the addresses it had before. A BLE link outlives the
    /// iroh connection on it until it is idle (about 25 s), and while a link
    /// to a phone's old address is up, BlueZ aborts every connect to its new
    /// one (`le-connection-abort-by-local`). Android changes the address when
    /// it leaves Add Device, so the first session after a pairing failed so.
    pub(crate) async fn close_links_to(&self, peer_key: iroh::EndpointId, after: Duration) {
        let Some((_, transport)) = self.endpoint.get().cloned() else {
            return;
        };
        tokio::time::sleep(after).await;
        log::info!("[transport][ble] closing the links to {peer_key}");
        transport.close_links(&peer_key);
    }

    /// A Bluetooth session with `peer_id` ended: close its BLE links, unless
    /// a newer session with the peer is held by then, or a Bluetooth setup
    /// with it is open (its connect may be on the same link: closing it then
    /// stopped the setup). A session that the peer's side dropped left the
    /// link up here, and iroh kept writing into it while the peer had moved
    /// to a new address.
    pub(crate) fn close_links_after_session(&self, peer_id: &str) {
        let Some(peer_key) = self.pinned_key(peer_id).and_then(|key| key.parse::<iroh::EndpointId>().ok()) else {
            return;
        };
        let bluetooth = self.arc();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(TIME_BEFORE_LINK_CLOSE).await;
            if bluetooth.has_session(&peer_id) || bluetooth.setup_open() {
                return;
            }
            bluetooth.close_links_to(peer_key, Duration::ZERO).await;
        });
    }

    /// A pairing with `peer_id` finished on the BLE link to `address`: keep the
    /// link for the first session (`remove_pairing_link`). Not used in
    /// `PAIRING_LINK_KEEP_TIME`, it is closed: a link left up blocks the connect
    /// to the peer's new address.
    pub(crate) fn keep_pairing_link(&self, peer_id: &str, address: &str, peer_key: iroh::EndpointId) {
        if let Ok(mut links) = self.pairing_links.lock() {
            links.insert(peer_id.to_string(), address.to_string());
        }
        // A new pair: rejected sessions and misses from before it (the other
        // device rejected them while it was not paired yet) must not stop its
        // first session.
        self.delivery.forget_delivery_misses(peer_id);
        log::info!("[transport][ble] keeping the pairing link to {address} for the first session");
        self.bus.publish(Event::DeviceHeard { peer_device_id: peer_id.to_string(), kind: ChannelKind::Bluetooth });
        let bluetooth = self.arc();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(PAIRING_LINK_KEEP_TIME).await;
            let unused = bluetooth.remove_pairing_link(&peer_id).is_some();
            if unused && !bluetooth.has_session(&peer_id) {
                bluetooth.close_links_to(peer_key, Duration::ZERO).await;
            }
        });
    }

    /// Whether a pairing link is kept for `peer_id`'s first session.
    pub(crate) fn has_pairing_link(&self, peer_id: &str) -> bool {
        self.pairing_links.lock().is_ok_and(|links| links.contains_key(peer_id))
    }

    /// The address of the pairing link kept for `peer_id`'s first session,
    /// once: a later session searches as usual.
    pub(crate) fn remove_pairing_link(&self, peer_id: &str) -> Option<String> {
        self.pairing_links.lock().ok()?.remove(peer_id)
    }

    /// The gate rejected a Bluetooth session from `address` (not paired, or
    /// the channel is off or missing). Its BLE link stayed up until idle, and
    /// iroh sent this device's own next connection to the peer into it
    /// instead of connecting: the connect timed out. No session to the peer is
    /// kept on this channel then, so the link is closed after the rejection
    /// gets to the peer -- unless a setup with `device_id` is open: its
    /// connect can use the same link.
    pub(crate) fn close_rejected_link(&self, address: String, device_id: Option<&str>) {
        if address.is_empty() {
            return;
        }
        let known_device = device_id.is_some();
        let bluetooth = self.arc();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(TIME_BEFORE_LINK_CLOSE).await;
            if known_device && bluetooth.setup_open() {
                return;
            }
            bluetooth.close_link_at(address, Duration::ZERO).await;
        });
    }

    /// Like `close_links_to`, for a link the peer opened from `address`,
    /// which this device never connected to.
    pub(crate) async fn close_link_at(&self, address: String, after: Duration) {
        let Some((_, transport)) = self.endpoint.get().cloned() else {
            return;
        };
        tokio::time::sleep(after).await;
        log::info!("[transport][ble] closing the link to {address}");
        transport.close_link(&PeerAddress(address));
    }

    /// Routes a channel a central opened by its first datagram: a QUIC
    /// Initial goes to the iroh transport (an authenticated session),
    /// anything else is one of Fini's own pre-pairing frames on a plain
    /// link.
    pub(super) async fn route_inbound(&self, mut channel: DatagramChannel) -> Option<tokio::task::JoinHandle<()>> {
        let first = match tokio::time::timeout(FIRST_DATAGRAM_TIMEOUT, channel.recv()).await {
            Ok(Some(Ok(first))) => first,
            _ => return None,
        };
        if ble_gatt_iroh::is_quic_initial(&first) {
            match self.iroh_endpoint().await {
                Ok((_, transport)) => Some(transport.attach_after(channel, first)),
                Err(err) => {
                    log::warn!("[transport][ble] cannot take a session: {err}");
                    None
                }
            }
        } else {
            let link: Box<dyn DataLink> = Box::new(BleDataLink::after(channel, first));
            let handlers = self.handlers.get().cloned()?;
            Some(tokio::spawn(async move { handlers.peer.accept(link).await }))
        }
    }
}
}

mod scanner {
//! The scanner: the one Bluetooth scan of the app, and what it heard.
//!
//! It gives the radio to one task at a time: a scan, a connection, an
//! identify step, or a pairing step. It also keeps what the scan heard:
//! the devices available for Add Device (`AvailableDevices`), and when and
//! where each paired device was last heard.
//!
//! One `Scanner` per Bluetooth channel, so two devices in one test process
//! each have their own.
//!
//! It runs the search coordinator (ADR-0008 D13): one scan for the
//! whole app. Callers ask for a search with a purpose -- status while the
//! Device page is open, delivery when there is work for a peer, setup while
//! the setup dialog is open -- and the coordinator merges whatever is asked
//! into one running scan over every peer wanted. It never starts the radio
//! twice for the same thing.
//!
//! What the scan hears:
//!
//! | heard | result |
//! |---|---|
//! | any wanted peer | its last-heard time is recorded (green, D9) |
//! | a peer a delivery or setup search waits for | its address is handed to that search, which dials it |
//!
//! Scanning and dialling never overlap: an adapter cannot usefully drive
//! discovery and establish a connection at once (hardware evidence in
//! `connect_by_advertisement`'s history). A handed-off address carries a
//! `ConnectGuard`, and the scan does not resume until every guard is dropped.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Once};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, watch, Mutex, MutexGuard, Notify};

use super::{connect, DataLink, ADD_MODE_FLAG_BYTE};
use super::{
    advertised_fingerprint, datagram_config, fingerprint_of, BluetoothAdapter, FINGERPRINT_LEN,
    FINI_MANUFACTURER_ID, SCAN_WINDOW,
};
use crate::models::Event;
use crate::services::channel_service::ChannelKind;
use crate::services::event_bus_service::EventBusService;

/// How long a heard advertisement counts as current (`heard_recently`).
const BLUETOOTH_CHANNEL_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a heard address gets a connect without a search first
/// (`last_heard_address`). An Android phone takes a new address each time it
/// advertises again (Add Device, a channel setup): an address heard 30 s
/// ago got a connect that waited until the 15 s timeout, while a search finds the
/// new one in 1-3 s.
const LAST_HEARD_CONNECT_TIME: Duration = Duration::from_secs(5);

/// How long an address whose dial just failed is not handed off again.
///
/// BlueZ keeps a device object for every address it has seen and reports it
/// again at the start of each discovery (`rssi=None`), including the phone's
/// previous private address. Without a cooldown the search heard that stale
/// address, dialled it, was refused after 2s, restarted discovery, heard the
/// same cached entry at once and dialled again -- a start/stop/dial loop every
/// 2s that ended with BlueZ answering every later `StartDiscovery` with
/// "Operation already in progress" and `StopDiscovery` with "No discovery
/// started", after which nothing could be found until bluetoothd restarted.
const FAILED_CONNECT_COOLDOWN: Duration = Duration::from_secs(10);

/// On Linux in Add Device mode: scan this long, then pause for
/// `ADD_DEVICE_SCAN_PAUSE`. BlueZ discovery scans without a gap, and while it
/// does, the phone heard this desktop's advertisement only once in about 3 s
/// (measured, Intel AX210); the old design paused its scan between passes.
const ADD_DEVICE_SCAN_WINDOW: Duration = Duration::from_secs(4);
const ADD_DEVICE_SCAN_PAUSE: Duration = Duration::from_secs(1);

/// How long the identify step tries its connect again
/// (`connect_heard_device`).
const IDENTIFY_TRY_AGAIN_TIME: Duration = Duration::from_secs(5);

/// Pause after the radio refused a scan, so a broken adapter is not hammered.
const SCAN_FAILURE_PAUSE: Duration = Duration::from_secs(5);

/// Why a search runs (ADR-0008 D12). The coordinator treats delivery and
/// setup alike -- hand the peer's address over -- and keeps the purpose for
/// the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Delivery,
    Setup,
}

/// A peer heard by a delivery or setup search. Scanning stays paused while
/// this lives, so the caller can dial without competing with the scan.
pub struct Found {
    pub address: String,
    _connect: ConnectGuard,
}

/// Held while a dial runs. The scan waits for every guard to be dropped.
pub struct ConnectGuard(Arc<watch::Sender<usize>>);

impl Drop for ConnectGuard {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// Held while a pairing leg or a Bluetooth channel setup runs; see
/// `Scanner::pairing_steps`.
pub struct PairingStep(Arc<watch::Sender<usize>>);

impl Drop for PairingStep {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// Marks a started discovery session; clears the mark when dropped.
pub(super) struct RunningScan<'a>(&'a AtomicBool);

impl Drop for RunningScan<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct Request {
    id: u64,
    peer: String,
    purpose: Purpose,
    reply: Option<oneshot::Sender<Found>>,
}

#[derive(Default)]
struct Requests {
    next_id: u64,
    searches: Vec<Request>,
}

pub struct Scanner {
    /// Pairing legs (request / accept / complete) in progress. The add-mode
    /// candidate scan yields to them: it holds a discovery session and
    /// re-dials the same phone every pass, and an adapter doing either while
    /// a pairing dial runs refuses or stalls it -- ble-gatt answered the pair
    /// dial with "a dial to this peer is already in flight" and one dial hung
    /// until abandoned, so the Pair button reported "Couldn't reach".
    pairing_steps: Arc<watch::Sender<usize>>,
    /// Held for the length of one candidate probe; see `connect_for_pairing`.
    candidate_probe: Mutex<()>,
    /// One Bluetooth scan at a time. Every scan holds this for
    /// as long as its discovery stream lives.
    ///
    /// Android's backend refuses a second scan while one is running ("a scan
    /// is already active"), and each caller read that refusal as "the adapter
    /// is unavailable" -- so the adapter check a person triggers by switching
    /// a channel on, landing inside the dial loop's scan window, reported
    /// their working Bluetooth as off, and the dial loop did the same in
    /// reverse. Linux has no such refusal, but an adapter driving two
    /// discovery sessions competes with itself (see
    /// `connect_by_advertisement`), so the lease applies on every platform.
    scan_lock: Mutex<()>,
    /// Set while a discovery session is actually running: `scan()` returned
    /// Ok and the stream is alive. Holding `scan_lock` is not enough -- the
    /// holder may still be waiting on an adapter that will refuse it.
    scan_running: AtomicBool,
    /// Peers with an exchange attempt in flight -- one at a time per peer.
    exchanges: StdMutex<HashSet<String>>,
    /// When this device's own connection to each peer became a session
    /// (`note_connected`).
    connected_at: StdMutex<HashMap<String, Instant>>,
    /// Dials running now; the coordinator's scan waits for none.
    connections: Arc<watch::Sender<usize>>,
    requests: StdMutex<Requests>,
    /// Wakes the scan loop whenever what is wanted changes.
    changed: Notify,
    started: Once,
    failed_connects: StdMutex<HashMap<String, Instant>>,
    /// When each paired device was last heard advertising, and at which
    /// address. Written only on a fingerprint match, so "heard" means "heard
    /// advertising as this device".
    paired_heard: StdMutex<HashMap<String, (Instant, String)>>,
    /// Set while Add Device is open; see `set_add_device_mode`.
    add_device_mode: AtomicBool,
    /// The devices around this one in Add Device mode.
    pub(crate) available: AvailableDevices,
    /// Told when a paired device is heard again.
    bus: Arc<EventBusService>,
}

#[cfg(test)]
impl Default for Scanner {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl Scanner {
    pub fn new(bus: Arc<EventBusService>) -> Self {
        Self {
            bus,
            pairing_steps: Arc::new(watch::channel(0).0),
            candidate_probe: Mutex::new(()),
            scan_lock: Mutex::new(()),
            scan_running: AtomicBool::new(false),
            exchanges: StdMutex::new(HashSet::new()),
            connected_at: StdMutex::new(HashMap::new()),
            connections: Arc::new(watch::channel(0).0),
            requests: StdMutex::new(Requests::default()),
            changed: Notify::new(),
            started: Once::new(),
            failed_connects: StdMutex::new(HashMap::new()),
            paired_heard: StdMutex::new(HashMap::new()),
            add_device_mode: AtomicBool::new(false),
            available: AvailableDevices::default(),
        }
    }

    /// Records that `peer_id` was heard advertising at `address`.
    fn record_paired_heard(&self, peer_id: &str, address: &str) {
        let newly_present = match self.paired_heard.lock() {
            Ok(mut heard) => heard
                .insert(peer_id.to_string(), (Instant::now(), address.to_string()))
                .is_none_or(|(previous, _)| previous.elapsed() >= BLUETOOTH_CHANNEL_TIMEOUT),
            Err(_) => false,
        };
        // A device that appears is the moment waiting work can go to it
        // (ADR-0008 D8). Only on that change: a device heard in every window
        // must not make SpaceSync poll.
        if newly_present {
            self.bus.publish(Event::DeviceHeard { peer_device_id: peer_id.to_string(), kind: ChannelKind::Bluetooth });
        }
    }

    /// Where `peer_id` was last heard advertising, if in
    /// `LAST_HEARD_CONNECT_TIME`: a device heard just now is reached by one
    /// connection, not a search.
    pub fn last_heard_address(&self, peer_id: &str) -> Option<String> {
        self.heard_in(peer_id, LAST_HEARD_CONNECT_TIME)
    }

    /// Whether `peer_id` advertised within the channel timeout (ADR-0008 D9).
    pub fn heard_recently(&self, peer_id: &str) -> bool {
        self.heard_in(peer_id, BLUETOOTH_CHANNEL_TIMEOUT).is_some()
    }

    fn heard_in(&self, peer_id: &str, window: Duration) -> Option<String> {
        let heard = self.paired_heard.lock().ok()?;
        heard
            .get(peer_id)
            .filter(|(at, _)| at.elapsed() < window)
            .map(|(_, address)| address.clone())
    }

    pub fn begin_pairing_step(&self) -> PairingStep {
        self.pairing_steps.send_modify(|count| *count += 1);
        PairingStep(self.pairing_steps.clone())
    }

    /// Pairing steps held right now.
    #[cfg(test)]
    pub(crate) fn pairing_steps_held(&self) -> usize {
        *self.pairing_steps.borrow()
    }

    /// Held for the length of one candidate probe. A dial to a peer the
    /// probe may be dialling waits for it, rather than cancelling it.
    pub async fn candidate_probe(&self) -> MutexGuard<'_, ()> {
        self.candidate_probe.lock().await
    }

    pub(super) async fn scan_lock(&self) -> MutexGuard<'_, ()> {
        self.scan_lock.lock().await
    }

    pub(super) fn try_scan_lock(&self) -> Option<MutexGuard<'_, ()>> {
        self.scan_lock.try_lock().ok()
    }

    pub(super) fn start_running_scan(&self) -> RunningScan<'_> {
        self.scan_running.store(true, Ordering::SeqCst);
        BluetoothAdapter::this_device().note_reachable();
        RunningScan(&self.scan_running)
    }

    pub(super) fn scan_is_running(&self) -> bool {
        self.scan_running.load(Ordering::SeqCst)
    }

    /// Whether this device is connecting to the peer right now, or its own
    /// connection became a session within `session::CROSSING_WINDOW`.
    pub fn connecting(&self, peer_id: &str) -> bool {
        self.exchanges.lock().unwrap().contains(peer_id)
            || self
                .connected_at
                .lock()
                .unwrap()
                .get(peer_id)
                .is_some_and(|at| at.elapsed() < crate::services::space_sync_service::CROSSING_WINDOW)
    }

    /// Registers an exchange attempt with `peer_id`; false if one runs.
    pub(super) fn begin_exchange(&self, peer_id: &str) -> bool {
        self.exchanges.lock().unwrap().insert(peer_id.to_string())
    }

    pub(super) fn end_exchange(&self, peer_id: &str) {
        self.exchanges.lock().unwrap().remove(peer_id);
    }

    /// This device's own connection to `peer_id` became a held session: it
    /// is no longer connecting, but for `CROSSING_WINDOW` a connection from
    /// the peer is still a crossing.
    pub(super) fn note_connected(&self, peer_id: &str) {
        self.connected_at.lock().unwrap().insert(peer_id.to_string(), Instant::now());
        self.end_exchange(peer_id);
    }

    fn connect_guard(&self) -> ConnectGuard {
        self.connections.send_modify(|count| *count += 1);
        ConnectGuard(self.connections.clone())
    }

    /// A guard for a dial the coordinator did not hand off: the running
    /// scan sees the guard and stops, and this returns once it has (the
    /// scan lease is free), so the dial never overlaps a scan.
    pub async fn acquire_connect(&self) -> ConnectGuard {
        let guard = self.connect_guard();
        drop(self.scan_lock().await);
        guard
    }

    fn start_once(self: &Arc<Self>) {
        // Unit tests drive `hand_off` directly; a real scan loop there would
        // reach for an adapter the test machine does not have.
        if cfg!(test) {
            return;
        }
        // Tauri's runtime, not `tokio::spawn`: this is reached from
        // synchronous commands, which run on the main thread outside any
        // Tokio context.
        self.started.call_once(|| {
            tauri::async_runtime::spawn(self.clone().run());
        });
    }

    /// Search for `peer` for up to `window`, ending early once it is heard.
    pub async fn find(self: &Arc<Self>, peer: &str, purpose: Purpose, window: Duration) -> Option<Found> {
        let (reply, heard) = oneshot::channel();
        let id = {
            let mut requests = self.requests.lock().unwrap();
            requests.next_id += 1;
            let id = requests.next_id;
            requests.searches.push(Request {
                id,
                peer: peer.to_string(),
                purpose,
                reply: Some(reply),
            });
            id
        };
        self.start_once();
        self.changed.notify_one();
        log::info!("[transport][ble] {purpose:?} search for {peer} started (up to {window:?})");
        let started = Instant::now();

        let found = tokio::time::timeout(window, heard).await.ok().and_then(Result::ok);

        self.requests.lock().unwrap().searches.retain(|request| request.id != id);
        self.changed.notify_one();
        log::info!(
            "[transport][ble] {purpose:?} search for {peer} ended after {:?}: {}",
            started.elapsed(),
            if found.is_some() { "heard" } else { "not heard" }
        );
        if found.is_some() {
            // The hand-off wakes this task before the coordinator has dropped
            // its discovery stream; the lease comes free only once it has, so
            // the caller's dial never overlaps the scan. `Found` holds a dial
            // guard, so no new scan starts meanwhile.
            drop(self.scan_lock().await);
        }
        found
    }

    /// Add Device opened or closed. While it is open, the scan runs without
    /// a stop, records every Fini device it hears (`AvailableDevices`), and
    /// serves only setup searches: delivery searches wait, so that the radio
    /// stays on the scan the person is watching.
    pub fn set_add_device_mode(self: &Arc<Self>, on: bool) {
        if on {
            self.available.remove_heard();
        }
        self.add_device_mode.store(on, Ordering::SeqCst);
        self.start_once();
        self.changed.notify_one();
    }

    fn in_add_device_mode(&self) -> bool {
        self.add_device_mode.load(Ordering::SeqCst)
    }

    /// Whether the scan has a reason to run.
    fn scan_wanted(&self, wanted: &HashMap<[u8; FINGERPRINT_LEN], String>) -> bool {
        !wanted.is_empty() || self.in_add_device_mode()
    }

    /// The peers the next scan listens for, by advertised fingerprint.
    fn wanted(&self) -> HashMap<[u8; FINGERPRINT_LEN], String> {
        let adding = self.in_add_device_mode();
        let requests = self.requests.lock().unwrap();
        requests
            .searches
            .iter()
            .filter(|request| !adding || request.purpose != Purpose::Delivery)
            .map(|request| (fingerprint_of(&request.peer), request.peer.clone()))
            .collect()
    }

    /// Hand `address` to every search waiting for `peer`. True if any was.
    fn hand_off(&self, peer: &str, address: &str) -> bool {
        let mut requests = self.requests.lock().unwrap();
        let mut handed = false;
        for request in requests.searches.iter_mut().filter(|request| request.peer == peer) {
            if let Some(reply) = request.reply.take() {
                log::info!("[transport][ble] {:?} search heard {peer} at {address}", request.purpose);
                handed |= reply
                    .send(Found {
                        address: address.to_string(),
                        _connect: self.connect_guard(),
                    })
                    .is_ok();
            }
        }
        handed
    }

    /// A dial to `address` after a hand-off failed: the search skips that
    /// address for `FAILED_CONNECT_COOLDOWN` and keeps listening for the peer's
    /// current one.
    pub fn note_connect_failed(&self, address: &str) {
        self.failed_connects.lock().unwrap().insert(address.to_string(), Instant::now());
    }

    fn connect_recently_failed(&self, address: &str) -> bool {
        let mut failed = self.failed_connects.lock().unwrap();
        failed.retain(|_, at| at.elapsed() < FAILED_CONNECT_COOLDOWN);
        failed.contains_key(address)
    }

    async fn run(self: Arc<Self>) {
        use futures_util::StreamExt;

        let mut connections = self.connections.subscribe();
        loop {
            // Never scan while a dial runs.
            let _ = connections.wait_for(|count| *count == 0).await;

            let mut wanted = self.wanted();
            if !self.scan_wanted(&wanted) {
                // Only a new search, or Add Device, changes anything.
                self.changed.notified().await;
                continue;
            }

            let bluetooth = BluetoothAdapter::this_device();
            let Ok(adapter) = bluetooth.adapter().await else {
                bluetooth.note_unreachable();
                let _ = tokio::time::timeout(SCAN_FAILURE_PAUSE, self.changed.notified()).await;
                continue;
            };
            let scan = self.scan_lock().await;
            let mut discovered = match adapter.central().scan(datagram_config().service).await {
                Ok(discovered) => discovered,
                Err(err) => {
                    log::warn!("[transport][ble] search scan refused: {err}");
                    bluetooth.note_scan_refused(&err);
                    drop(scan);
                    let _ = tokio::time::timeout(SCAN_FAILURE_PAUSE, self.changed.notified()).await;
                    continue;
                }
            };
            let running = self.start_running_scan();
            log::info!(
                "[transport][ble] shared scan window started for {} device(s), add device {}",
                wanted.len(),
                if self.in_add_device_mode() { "on" } else { "off" }
            );

            // One window at a time: the wanted set is re-read between
            // windows, and at once whenever a search starts or ends. On Linux
            // in Add Device mode the windows are short with a pause between
            // them (`ADD_DEVICE_SCAN_WINDOW`), so this device's own
            // advertisement gets air time.
            let window = if self.in_add_device_mode() && cfg!(target_os = "linux") {
                ADD_DEVICE_SCAN_WINDOW
            } else {
                SCAN_WINDOW
            };
            let deadline = tokio::time::Instant::now() + window;
            loop {
                tokio::select! {
                    next = tokio::time::timeout_at(deadline, discovered.next()) => match next {
                        Ok(Some(Ok(candidate))) => {
                            let payload = candidate
                                .manufacturer_data
                                .get(&FINI_MANUFACTURER_ID)
                                .map(|payload| payload.as_slice());
                            let advertised = advertised_fingerprint(payload);
                            if self.in_add_device_mode() {
                                if let Some(fingerprint) = advertised {
                                    let add_device_flag = payload
                                        .and_then(|payload| payload.first())
                                        .is_some_and(|flags| flags & ADD_MODE_FLAG_BYTE != 0);
                                    // The UI reads the list again only when
                                    // it changes (ADR-0010 D9).
                                    if self.available.record_heard(
                                        fingerprint,
                                        &candidate.address.0,
                                        candidate.name.clone(),
                                        add_device_flag,
                                    ) {
                                        self.bus.publish(Event::AvailableDevicesChanged);
                                    }
                                }
                            }
                            let Some(peer) = advertised.and_then(|fp| wanted.get(&fp)) else {
                                continue;
                            };
                            self.record_paired_heard(peer, &candidate.address.0);
                            if self.connect_recently_failed(&candidate.address.0) {
                                continue;
                            }
                            if self.hand_off(peer, &candidate.address.0) {
                                break;
                            }
                        }
                        Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                    },
                    // A dial started outside a hand-off: stop for it.
                    _ = connections.wait_for(|count| *count > 0) => break,
                    // What is wanted changed: keep the scan running over the
                    // new set rather than restarting the radio, and stop only
                    // once nothing is wanted any more.
                    _ = self.changed.notified() => {
                        wanted = self.wanted();
                        if !self.scan_wanted(&wanted) {
                            break;
                        }
                    }
                }
            }
            // Stop discovery before any dial the hand-off starts.
            drop(running);
            drop(discovered);
            drop(scan);
            if self.in_add_device_mode() && cfg!(target_os = "linux") {
                let _ = tokio::time::timeout(ADD_DEVICE_SCAN_PAUSE, self.changed.notified()).await;
            }
        }
    }
}

// ---- A device heard in Add Device ----

impl Scanner {
    /// One plain link (no key yet) to the newest address the scan heard the
    /// device with `fingerprint` at: what pairing's identify step talks over
    /// (`DeviceConnectionState::identify_heard_device`). Returns the link,
    /// the address, and guards that keep any scan paused until dropped.
    /// The connection is never cut short: ble-gatt gives it its own 20 s,
    /// and an abandoned connect makes BlueZ crash (two crashes in the spike).
    ///
    /// When a connect ends with an error, it is tried again, to the newest
    /// heard address, until `IDENTIFY_TRY_AGAIN_TIME` after the first try.
    /// Right after a pair is removed, the release of the old link made BlueZ
    /// cancel the first connect (`le-connection-abort-by-local`), and the
    /// pairing stopped.
    pub async fn connect_heard_device(
        &self, fingerprint: [u8; FINGERPRINT_LEN],
    ) -> Result<(Box<dyn DataLink>, String, impl Sized + '_), String> {
        let device_gone = || "that device is not heard any more -- open Add Device on it again".to_string();
        // A search for a paired device may start a scan; holding a connect
        // guard keeps it paused until the caller is done.
        let probe = self.candidate_probe().await;
        let connect_guard = self.acquire_connect().await;
        let started = Instant::now();
        loop {
            let address = self.available.heard_address(fingerprint).ok_or_else(device_gone)?;
            match connect(&address).await {
                Ok(link) => return Ok((link, address, (probe, connect_guard))),
                Err(err) if started.elapsed() < IDENTIFY_TRY_AGAIN_TIME => {
                    log::info!("[transport][ble] connect to {address} ended with an error ({err}); trying again");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(err) => {
                    return Err(format!("the device did not answer ({err}) -- make sure Add Device is open on it"))
                }
            }
        }
    }
}

/// A Fini device near this one that is in Add Device mode, as an identify
/// step learned it.
#[derive(Clone)]
pub struct AvailableDevice {
    pub address: String,
    pub device_id: String,
    pub hostname: String,
}

// ---- Devices available for Add Device ----

/// The Fini devices around this one while Add Device is open: what their
/// advertisements told the scan, and what an identify step learned.
#[derive(Default)]
pub struct AvailableDevices {
    /// Every Fini device the scan heard while Add Device is open, by
    /// advertised fingerprint: from its advertisement alone, with no
    /// connection.
    heard: StdMutex<HashMap<[u8; FINGERPRINT_LEN], HeardDevice>>,
    /// What identify steps learned, by device id: the device, and the iroh
    /// key that a pair request connects to.
    identified: StdMutex<HashMap<String, (AvailableDevice, String)>>,
}

/// How long a device stays in the Add Device list after the scan last heard
/// it with the Add Device flag. A phone advertises several times a second, so
/// a device that stops (it left Add Device, or went away) drops out quickly.
const HEARD_FRESH: Duration = Duration::from_secs(15);

/// What one advertisement told the scan.
#[derive(Clone)]
struct HeardDevice {
    address: String,
    name: Option<String>,
    add_device_flag: bool,
    at: Instant,
}

/// A device for the Add Device list, from the advertisement alone.
#[derive(Clone, Debug)]
pub struct HeardInAddDevice {
    pub fingerprint: [u8; FINGERPRINT_LEN],
    pub address: String,
    /// The short name from the scan response, if the device sent one.
    pub name: Option<String>,
    /// The device id and full name, if an identify step learned them.
    pub identified: Option<(String, String)>,
}

impl AvailableDevices {
    /// Forgets what the scan heard, so that a new Add Device starts empty:
    /// a device listed now must be in Add Device now.
    pub fn remove_heard(&self) {
        if let Ok(mut heard) = self.heard.lock() {
            heard.clear();
        }
    }

    /// Records one advertisement: the newest address of that device wins.
    /// Returns true when the Add Device list changed: the device came into
    /// it, left it, or got its name.
    pub fn record_heard(
        &self, fingerprint: [u8; FINGERPRINT_LEN], address: &str, name: Option<String>, add_device_flag: bool,
    ) -> bool {
        let Ok(mut heard) = self.heard.lock() else { return false };
        let known = heard.get(&fingerprint);
        let listed_before = known.is_some_and(|known| known.add_device_flag && known.at.elapsed() < HEARD_FRESH);
        let name_before = known.and_then(|known| known.name.clone());
        let name = name.or_else(|| name_before.clone());
        let changed = listed_before != add_device_flag || (add_device_flag && name != name_before);
        heard.insert(
            fingerprint,
            HeardDevice { address: address.to_string(), name, add_device_flag, at: Instant::now() },
        );
        changed
    }

    /// The devices for the Add Device list: heard with the Add Device flag
    /// in the last `HEARD_FRESH`, without this device itself.
    pub fn in_add_device(&self, mine: [u8; FINGERPRINT_LEN]) -> Vec<HeardInAddDevice> {
        let heard: Vec<([u8; FINGERPRINT_LEN], HeardDevice)> = match self.heard.lock() {
            Ok(heard) => heard
                .iter()
                .filter(|(fingerprint, device)| {
                    **fingerprint != mine && device.add_device_flag && device.at.elapsed() < HEARD_FRESH
                })
                .map(|(fingerprint, device)| (*fingerprint, device.clone()))
                .collect(),
            Err(_) => return Vec::new(),
        };
        let identified: Vec<(String, String)> = match self.identified.lock() {
            Ok(identified) => identified
                .values()
                .map(|(device, _)| (device.device_id.clone(), device.hostname.clone()))
                .collect(),
            Err(_) => Vec::new(),
        };
        heard
            .into_iter()
            .map(|(fingerprint, device)| HeardInAddDevice {
                fingerprint,
                address: device.address,
                name: device.name,
                identified: identified.iter().find(|(id, _)| fingerprint_of(id) == fingerprint).cloned(),
            })
            .collect()
    }

    /// The newest address the scan heard `fingerprint` at, if fresh.
    pub fn heard_address(&self, fingerprint: [u8; FINGERPRINT_LEN]) -> Option<String> {
        let heard = self.heard.lock().ok()?;
        heard.get(&fingerprint).filter(|device| device.at.elapsed() < HEARD_FRESH).map(|device| device.address.clone())
    }

    /// The iroh key `device_id` reported, if an identify step learned it.
    pub fn key(&self, device_id: &str) -> Option<String> {
        self.identified.lock().ok()?.get(device_id).map(|(_, key)| key.clone())
    }

    /// The newest address of `device_id`: where the scan last heard it, or
    /// else where it was identified. Android changes its address on every
    /// Add Device change, so the address a list row was built from can be
    /// gone when the person presses Pair.
    pub fn latest_address(&self, device_id: &str) -> Option<String> {
        self.heard_address(fingerprint_of(device_id)).or_else(|| {
            self.identified.lock().ok()?.get(device_id).map(|(device, _)| device.address.clone())
        })
    }

    /// Records what an identify step learned.
    pub fn record_identified(&self, device: &AvailableDevice, endpoint_id: String) {
        if let Ok(mut identified) = self.identified.lock() {
            identified.insert(device.device_id.clone(), (device.clone(), endpoint_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searching(radio: &Scanner, peer: &str) -> bool {
        radio.requests.lock().unwrap().searches.iter().any(|request| request.peer == peer)
    }

    /// A connection started outside a hand-off waits until the scan holding
    /// the adapter has stopped, and the scan sees the connection.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_connection_waits_for_the_running_scan_to_stop() {
        let radio = Arc::new(Scanner::default());
        let scan = radio.scan_lock().await;
        let connection = tokio::spawn({
            let radio = radio.clone();
            async move { radio.acquire_connect().await }
        });
        while *radio.connections.borrow() == 0 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!connection.is_finished(), "no connection while the scan holds the adapter");

        drop(scan);
        let guard = tokio::time::timeout(Duration::from_secs(2), connection).await.unwrap().unwrap();
        drop(guard);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn searches_for_different_peers_share_one_scan() {
        let radio = Arc::new(Scanner::default());
        let first = tokio::spawn({
            let radio = radio.clone();
            async move { radio.find("coord-merge-a", Purpose::Delivery, Duration::from_secs(5)).await }
        });
        let second = tokio::spawn({
            let radio = radio.clone();
            async move { radio.find("coord-merge-b", Purpose::Setup, Duration::from_secs(5)).await }
        });
        while !(searching(&radio, "coord-merge-a") && searching(&radio, "coord-merge-b")) {
            tokio::task::yield_now().await;
        }

        let wanted = radio.wanted();
        assert_eq!(wanted.get(&fingerprint_of("coord-merge-a")).map(String::as_str), Some("coord-merge-a"));
        assert_eq!(wanted.get(&fingerprint_of("coord-merge-b")).map(String::as_str), Some("coord-merge-b"));

        assert!(radio.hand_off("coord-merge-a", "AA:00:00:00:00:01"));
        assert!(radio.hand_off("coord-merge-b", "AA:00:00:00:00:02"));
        assert_eq!(first.await.unwrap().map(|found| found.address).as_deref(), Some("AA:00:00:00:00:01"));
        assert_eq!(second.await.unwrap().map(|found| found.address).as_deref(), Some("AA:00:00:00:00:02"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_heard_peer_reaches_every_search_waiting_for_it_and_pauses_the_scan() {
        let radio = Arc::new(Scanner::default());
        let delivery = tokio::spawn({
            let radio = radio.clone();
            async move { radio.find("coord-both", Purpose::Delivery, Duration::from_secs(5)).await }
        });
        let setup = tokio::spawn({
            let radio = radio.clone();
            async move { radio.find("coord-both", Purpose::Setup, Duration::from_secs(5)).await }
        });
        while radio.requests.lock().unwrap().searches.iter().filter(|r| r.peer == "coord-both").count() < 2 {
            tokio::task::yield_now().await;
        }

        assert!(radio.hand_off("coord-both", "AA:00:00:00:00:03"));
        let delivery = delivery.await.unwrap().expect("delivery search hears the peer");
        let setup = setup.await.unwrap().expect("setup search hears the peer");
        assert_eq!(delivery.address, "AA:00:00:00:00:03");
        assert_eq!(setup.address, "AA:00:00:00:00:03");
        // Scanning waits while either dial is possible.
        assert_eq!(*radio.connections.borrow(), 2);

        drop(delivery);
        drop(setup);
        assert!(!searching(&radio, "coord-both"), "a finished search withdraws itself");
    }

    /// A dial that just failed keeps that address out of hand-offs, and only
    /// until the cooldown has passed: the peer's next private address, or the
    /// same one once it answers again, is dialled as usual.
    #[test]
    fn a_failed_address_is_skipped_until_its_cooldown_expires() {
        let radio = Scanner::default();
        radio.note_connect_failed("AA:00:00:00:00:10");
        assert!(radio.connect_recently_failed("AA:00:00:00:00:10"));
        assert!(!radio.connect_recently_failed("AA:00:00:00:00:11"), "only the failed address is skipped");

        let expired = Instant::now()
            .checked_sub(FAILED_CONNECT_COOLDOWN)
            .expect("the clock is past one cooldown");
        radio.failed_connects.lock().unwrap().insert("AA:00:00:00:00:10".to_string(), expired);
        assert!(!radio.connect_recently_failed("AA:00:00:00:00:10"), "the cooldown has passed");
        assert!(
            !radio.failed_connects.lock().unwrap().contains_key("AA:00:00:00:00:10"),
            "an expired entry is dropped"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_search_that_hears_nothing_ends_with_its_window_and_withdraws() {
        let radio = Arc::new(Scanner::default());
        let found = radio.find("coord-silent", Purpose::Delivery, Duration::from_millis(50)).await;
        assert!(found.is_none());
        assert!(!searching(&radio, "coord-silent"));
        assert!(!radio.hand_off("coord-silent", "AA:00:00:00:00:04"));
    }
}
}
