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

#[cfg(any(feature = "ui-plane", test))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use ble_gatt::drivers::linux::LinuxBackend;
use ble_gatt::datagram::{self, DatagramChannel, DatagramConfig};
use ble_gatt::{Adapter, CharacteristicUuid, PeerAddress, ServiceUuid};
use uuid::Uuid;

use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::{
    bluetooth_connect_candidates, note_observed_bluetooth_address as store_observed_bluetooth_address,
    DeviceConnectionState, DeviceIdentity,
};
use crate::services::communication::sync::session;
use crate::services::communication::sync::space_sync::SpaceSync;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::sync::types::PeerFrame;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::channel::{recv_frame, send_frame};
use crate::services::communication::channel::iroh_link::{IrohDataLink, PEER_ALPN, SYNC_ALPN};
use crate::services::communication::channel::service::ChannelService;
use crate::services::communication::channel::{BoxConnectFuture, DataLink, Transport, ChannelKind};
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::pairing::ChannelProblem;

mod adapter;
mod advertiser;
mod data_link;
mod endpoint;
mod scanner;

pub use adapter::BluetoothAdapter;
pub use data_link::*;

pub use advertiser::Advertiser;
pub use scanner::Scanner;

use scanner::Purpose;

/// The Bluetooth channel of one `DeviceConnectionState`. It owns the scan
/// and what the scan heard, the advertisement, the delivery retry schedule
/// and the iroh endpoint over Bluetooth. What it needs but does not own --
/// the state's sessions, setups and database -- is given to each call.
pub struct BluetoothChannel {
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
}

impl BluetoothChannel {
    pub(crate) fn new(identity: &DeviceIdentity, space_sync: Arc<SpaceSync>) -> Self {
        Self {
            scanner: Arc::new(Scanner::new(space_sync)),
            advertiser: Arc::new(Advertiser::new(identity)),
            delivery: DeliverySchedule::default(),
            endpoint: tokio::sync::OnceCell::new(),
        }
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

/// Per paired device: how many delivery searches in a row found nothing,
/// and when the next one is due. In memory only, as ADR-0008 decided: a
/// started app tries at once.
#[derive(Default)]
pub struct DeliverySchedule {
    schedule: std::sync::Mutex<std::collections::HashMap<String, (usize, std::time::Instant)>>,
}

impl DeliverySchedule {
    /// Whether a delivery search for this device is due. A device heard
    /// recently is always due: it is reached by one connection.
    pub fn delivery_due(&self, scanner: &Scanner, peer_id: &str) -> bool {
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

    pub fn note_delivery_missed(&self, peer_id: &str, space_sync: &SpaceSync) {
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
        // The next search is due then; wake the keeper for it (ADR-0008 D12).
        space_sync.wake_after(next.saturating_duration_since(now));
    }

    /// The device was reached, or see `ChannelService::forget_failures`: the
    /// next delivery search is due at once. Also used when a pair is removed.
    pub fn forget_delivery_misses(&self, peer_id: &str) {
        if let Ok(mut schedule) = self.schedule.lock() {
            schedule.remove(peer_id);
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

const FINGERPRINT_LEN: usize = 4;

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
#[cfg(any(feature = "ui-plane", test))]
const ADD_MODE_FLAG_BYTE: u8 = 0x01;

/// `Transport` implementation for the Bluetooth adapter — see the note on
/// `channel::tcp_ws::TcpWsTransport` for why production dial loops call
/// `dial()` directly rather than through this trait object.
#[allow(dead_code)]
pub struct BleTransport;

#[async_trait]
impl Transport for BleTransport {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn connect(&self, _peer_device_id: &str, addr: &str, _port: u16) -> BoxConnectFuture {
        let addr = addr.to_string();
        Box::pin(async move { connect(&addr).await })
    }
}

/// The Bluetooth half of ADR-0008 D10/D12: presence, search and exchanges.
///
/// - **Presence**: a paired peer whose advertisement was heard within
///   `BLUETOOTH_CHANNEL_TIMEOUT` is present (green).
/// - **Search** is one coordinator (`search`, ADR-0008 D13) serving every
///   purpose with a single scan:
///   - status, while the Device page is open, keeps presence current;
///   - delivery, when there is work for a peer, searches up to
///     `SEARCH_WINDOW` and connects as soon as it is heard; a peer not found
///     is tried again after `DELIVERY_RETRY` steps, until it appears;
///   - setup, while the setup dialog is open, says hello to the peer.
/// - **Exchange**: connect, authenticate, push and acknowledge, then close
///   once idle (`session::run_session`).
///
/// Nothing here runs when there is no reason to: no work, no open page, no
/// setup -- the radio only advertises and listens for connections.
const SEARCH_WINDOW: Duration = Duration::from_secs(60);

/// Per-candidate cap for one dial + authentication inside a delivery search:
/// an advertiser that accepts the connection and never answers must not
/// hold the whole window.
const CONNECT_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one status-search window listens while the Device page is open.
const STATUS_SEARCH_WINDOW: Duration = Duration::from_secs(20);

#[async_trait]
impl ChannelService for BluetoothChannel {
    /// Linux and Android: the only builds with a Bluetooth channel.
    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool {
        true
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn usable(&self) -> bool {
        !BluetoothAdapter::this_device().is_unavailable()
    }

    #[cfg(any(feature = "ui-plane", test))]
    async fn probe(&self, _state: &DeviceConnectionState) -> bool {
        BluetoothAdapter::this_device().probe(&self.scanner).await
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self, state: &DeviceConnectionState) {
        // Linux only, deliberately. Android starts the same loop from
        // `keep_serving`, once its Activity context exists -- starting it
        // here as well gave Android two accept loops racing each other.
        #[cfg(target_os = "linux")]
        {
            let state = state.clone();
            tauri::async_runtime::spawn(async move {
                let db_path = state.db_path.clone();
                state.bluetooth.clone().run_server(state, db_path).await
            });
        }
        #[cfg(not(target_os = "linux"))]
        let _ = state;
    }

    fn keep_serving(&self, state: &DeviceConnectionState) {
        // Android starts its peripheral lazily, from the first real tick:
        // the Activity context it needs does not exist when the app boots
        // (see `start_peripheral_once`). Gated on the Nearby Devices
        // permission, because this runs from a background tick rather than
        // a user action -- without it `startAdvertising` throws
        // SecurityException and the loop retries forever.
        #[cfg(target_os = "android")]
        if crate::services::ble_plugin::permission_granted() {
            self.start_peripheral_once(state);
        }
        self.refresh_advertising(state);
    }

    fn request_exchange(&self, state: &DeviceConnectionState, peer_device_id: &str) {
        self.start_exchange(state, peer_device_id);
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn forget_failures(&self, peer_device_id: &str) {
        self.delivery.forget_delivery_misses(peer_device_id);
    }

    /// Turn the status search on while the Device page is open, off when it
    /// closes. Green is never computed in the background (ADR-0008 D12);
    /// the search coordinator merges it with any delivery or setup search
    /// running.
    #[cfg(any(feature = "ui-plane", test))]
    fn watch_presence(&self, state: &DeviceConnectionState, active: bool) {
        self.scanner.set_status(active.then(|| state.db_path.clone()));
    }

    fn is_present(&self, _state: &DeviceConnectionState, peer_device_id: &str) -> bool {
        self.scanner.heard_recently(peer_device_id)
    }

    /// One search of up to `SETUP_ROUND`, with a hello the moment the peer
    /// is heard. A round the radio refused pauses
    /// `SETUP_ROUND_FAILURE_PAUSE`, so a broken radio is not hammered.
    #[cfg(any(feature = "ui-plane", test))]
    async fn setup_round(&self, state: &DeviceConnectionState, peer_device_id: &str) -> bool {
        match self.find_and_send_request(state, peer_device_id, SETUP_ROUND).await {
            Ok(acknowledged) => acknowledged,
            Err(err) => {
                log::warn!("[setup] bluetooth round for {peer_device_id} failed: {err}");
                tokio::time::sleep(SETUP_ROUND_FAILURE_PAUSE).await;
                false
            }
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn connecting(&self, peer_device_id: &str) -> bool {
        self.scanner.connecting(peer_device_id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn problem(&self, _state: &DeviceConnectionState) -> Option<ChannelProblem> {
        BluetoothAdapter::this_device()
            .is_unavailable()
            .then_some(ChannelProblem::BluetoothUnavailable)
    }
}

impl BluetoothChannel {
    /// Starts the Bluetooth peripheral acceptor loop exactly once.
    /// Android-only: on Linux `start_serving` spawns `run_server` at
    /// startup, which is safe there since `LinuxBackend::new()` has no
    /// Android-context ordering requirement. On Android that same eager
    /// spawn would race `tao`'s own context bring-up, so the first call
    /// instead comes from `space_sync_tick_impl` — a `#[tauri::command]`,
    /// whose first real invocation can only happen once the
    /// WebView/Activity has actually dispatched an IPC call, a strictly
    /// later and safer point than anything obtainable from `.setup()`
    /// itself. The running check is a cheap filter -- this runs from every
    /// tick, and without it each one would spawn a task only for
    /// `Advertiser::claim_peripheral_role` to turn it away. The claim in
    /// `run_server` is the actual invariant; this just keeps the common path
    /// quiet.
    #[cfg(target_os = "android")]
    fn start_peripheral_once(&self, state: &DeviceConnectionState) {
        if !self.advertiser.peripheral_running() {
            let state = state.clone();
            tauri::async_runtime::spawn(async move {
                let db_path = state.db_path.clone();
                state.bluetooth.clone().run_server(state, db_path).await
            });
        }
    }

    /// Peripheral role: advertise Fini's BLE service and gate every
    /// accepted central through the same transport-neutral session gate
    /// every other adapter uses. No-op (logs and returns) when the local
    /// adapter can't do peripheral mode, or isn't available at all —
    /// Bluetooth is always a fallback, never a hard requirement to start the
    /// app. `ui-plane`/`test` only — `cli-plane` dials out but does not run
    /// an inbound acceptor.
    #[cfg(any(feature = "ui-plane", test))]
    pub async fn run_server(&self, state: DeviceConnectionState, db_path: PathBuf) {
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
                        let state = state.clone();
                        let db_path = db_path.clone();
                        live_centrals.fetch_add(1, Ordering::SeqCst);
                        let live_centrals = live_centrals.clone();
                        let central_gone_tx = central_gone_tx.clone();
                        // The central counts as live until its channel
                        // closes: the gate finishing on a plain link, or the
                        // iroh transport closing the channel of a session.
                        tokio::spawn(async move {
                            let bluetooth = state.bluetooth.clone();
                            // The device connected to us is the one dialling:
                            // the add-mode scan must not dial back while it
                            // is here. A second, crossing connection to the
                            // same device is what kept dropping both
                            // (initiator = central).
                            let _step = bluetooth.scanner.begin_pairing_step();
                            if let Some(handled) = bluetooth.route_inbound(&state, db_path, channel).await {
                                let _ = handled.await;
                            }
                            live_centrals.fetch_sub(1, Ordering::SeqCst);
                            let _ = central_gone_tx.send(());
                        });
                    }
                    _ = add_mode_rx.changed() => {
                        log::info!("[transport][ble] add-mode changed; re-advertising");
                        restarting_for_add_mode_change = true;
                        break;
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
    pub fn start_exchange(&self, state: &DeviceConnectionState, peer_id: &str) {
        if state.has_session_on(peer_id, ChannelKind::Bluetooth) || !self.delivery.delivery_due(&self.scanner, peer_id) {
            return;
        }
        // While a device is being added, the adapter belongs to that search.
        if self.advertiser.in_add_mode() {
            return;
        }
        if !self.scanner.begin_exchange(peer_id) {
            return;
        }
        let state = state.clone();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            let bluetooth = state.bluetooth.clone();
            bluetooth.exchange_with(&state, &peer_id).await;
            bluetooth.scanner.end_exchange(&peer_id);
        });
    }

    /// One delivery (ADR-0008 D8, D12): a present peer is one dial to where
    /// it was last heard; otherwise a delivery search of up to
    /// `SEARCH_WINDOW` connects the moment the peer is heard. Then
    /// authenticate and run the exchange to its idle end.
    async fn exchange_with(&self, state: &DeviceConnectionState, peer_id: &str) {
        let db_path = state.db_path.clone();
        if !is_still_bluetooth_eligible(&db_path, peer_id) {
            return;
        }
        let deadline = tokio::time::Instant::now() + SEARCH_WINDOW;
        let mut last_error = None;

        if let Some(address) = self.scanner.last_heard_address(peer_id) {
            let guard = self.scanner.acquire_connect().await;
            match self.connect_for_session(state, peer_id, &address).await {
                Ok(link) => {
                    drop(guard);
                    return self.run_exchange(state, peer_id, link, &address).await;
                }
                Err(err) => last_error = Some(err),
            }
        }

        loop {
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
                self.delivery.note_delivery_missed(peer_id, &state.space_sync);
                return;
            };
            match self.connect_for_session(state, peer_id, &found.address).await {
                Ok(link) => {
                    let address = found.address.clone();
                    drop(found);
                    return self.run_exchange(state, peer_id, link, &address).await;
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
    async fn connect_for_session(
        &self, state: &DeviceConnectionState, peer_id: &str, address: &str,
    ) -> Result<Box<dyn DataLink>, String> {
        let attempt = tokio::time::timeout(
            CONNECT_CANDIDATE_TIMEOUT,
            self.connect_session(state, peer_id, address, SYNC_ALPN),
        )
        .await;
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

    async fn run_exchange(
        &self, state: &DeviceConnectionState, peer_id: &str, link: Box<dyn DataLink>, address: &str,
    ) {
        self.delivery.forget_delivery_misses(peer_id);
        log::info!("[transport][ble] exchange with {peer_id} via {address}");
        let db_path = state.db_path.clone();
        // The switch could have been turned off during the search.
        if !is_still_bluetooth_eligible(&db_path, peer_id) {
            return;
        }
        // Diagnostics only: nothing dials a stored address.
        note_observed_bluetooth_address(&db_path, peer_id, address);
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        if state.try_claim_session(peer_id, ChannelKind::Bluetooth, tx) {
            let close_code = session::run_session(
                link,
                rx,
                state.clone(),
                db_path,
                peer_id.to_string(),
                session::idle_after(ChannelKind::Bluetooth),
            )
            .await;
            // Refused by the peer's gate: the next delivery waits its turn.
            if session::refused_by_peer(close_code) {
                self.delivery.note_delivery_missed(peer_id, &state.space_sync);
            }
        }
    }

    /// Recompute whether to advertise, from the stored channels and the
    /// setups running now. Cheap; called whenever one of its inputs may have
    /// changed.
    pub fn refresh_advertising(&self, state: &DeviceConnectionState) {
        let any_channel_on = tokio::task::block_in_place(|| {
            let mut conn = open_db_at_path(&state.db_path);
            !crate::services::communication::pairing::channels::peers_with_channel_enabled(
                &mut conn,
                ChannelKind::Bluetooth,
            )
            .is_empty()
        });
        let advertiser = &self.advertiser;
        let wanted = any_channel_on
            || state.any_channel_setup(ChannelKind::Bluetooth)
            || advertiser.answering_after_setup()
            || advertiser.in_add_mode();
        advertiser.set_wanted(wanted);
    }
}

/// Re-reads `paired_devices` for the current, live answer to "is this peer
/// still a valid Bluetooth dial target" — which since ADR-0006 means only
/// "is Bluetooth still enabled for this pair". `block_in_place` around the
/// blocking DB open, matching `sync::session::check_paired`'s existing
/// pattern for the same kind of call from inside an async loop.
fn is_still_bluetooth_eligible(db_path: &std::path::Path, peer_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        bluetooth_connect_candidates(&mut conn).iter().any(|candidate_id| candidate_id == peer_id)
    })
}

fn note_observed_bluetooth_address(db_path: &std::path::Path, peer_id: &str, address: &str) {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        store_observed_bluetooth_address(&mut conn, peer_id, address);
    })
}

// ---- Channel setup ----

/// How long one Bluetooth setup round listens before the next one starts
/// (ADR-0008 D12: a search lasts up to 60s and ends early on success).
#[cfg(all(feature = "ui-plane", not(test)))]
const SETUP_ROUND: Duration = Duration::from_secs(60);
#[cfg(test)]
const SETUP_ROUND: Duration = Duration::from_millis(200);

/// Pause after a Bluetooth round fails outright (scan refused, adapter
/// gone), so a broken radio is not hammered.
#[cfg(all(feature = "ui-plane", not(test)))]
const SETUP_ROUND_FAILURE_PAUSE: Duration = Duration::from_secs(5);
#[cfg(test)]
const SETUP_ROUND_FAILURE_PAUSE: Duration = Duration::from_millis(200);

/// How long the device with the higher id waits, in a Bluetooth setup, for the
/// other device's hello to reach it before dialling anyway. See
/// `BluetoothChannel::find_and_send_request`.
#[cfg(all(feature = "ui-plane", not(test)))]
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
#[cfg(any(feature = "ui-plane", test))]
const FIND_PEER_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg(any(feature = "ui-plane", test))]
impl BluetoothChannel {
    /// One Bluetooth setup search for `peer_id` (ADR-0008 D12): asks the
    /// scanner for the peer for up to `timeout`, and says hello the moment
    /// it is heard. `Ok(true)` if the peer acknowledged; `Err` when the
    /// radio itself is unusable, so the caller can pause.
    pub async fn find_and_send_request(
        &self, state: &DeviceConnectionState, peer_id: &str, timeout: Duration,
    ) -> Result<bool, String> {
        let scanner = &self.scanner;
        let Some(found) = scanner.find(peer_id, Purpose::Setup, timeout).await else {
            if BluetoothAdapter::this_device().is_unavailable() {
                return Err("bluetooth adapter unavailable".to_string());
            }
            return Ok(false);
        };
        // Both devices search for each other, and each dials the other the
        // moment it hears it. Two dials crossing fail each other: a device
        // that is itself mid-connect is not connectable, so the other's dial
        // hangs until BlueZ aborts it ("le-connection-abort-by-local" after
        // 8-12 s), and the retry crosses again. Measured on a laptop and a
        // Pixel, this was most of the 40-120 s a setup took. So the device
        // with the higher id lets the other dial first, and dials back once
        // the other's hello has reached it.
        self.wait_for_turn_to_connect(state, peer_id).await;
        // A candidate probe already dialling is let finish, not cancelled;
        // this dial must not overlap one to the same peer.
        let _probe = scanner.candidate_probe().await;
        // Bounded: a candidate that accepts the connection and never answers
        // must not hold the round open.
        let acknowledged = tokio::time::timeout(
            FIND_PEER_CANDIDATE_TIMEOUT,
            self.send_request(state, &found.address, peer_id),
        )
        .await
        .ok()
        .flatten()
        .is_some();
        if !acknowledged {
            scanner.note_connect_failed(&found.address);
        }
        Ok(acknowledged)
    }

    /// In a Bluetooth setup the device with the lower id dials first; the
    /// one with the higher id waits until the other's hello has reached it
    /// (or `HIGHER_ID_PATIENCE` runs out) before dialling back.
    pub(crate) async fn wait_for_turn_to_connect(&self, state: &DeviceConnectionState, peer_id: &str) {
        if state.identity.device_id.as_str() <= peer_id {
            return;
        }
        let patience = tokio::time::Instant::now() + HIGHER_ID_PATIENCE;
        while tokio::time::Instant::now() < patience
            && !state
                .channel_setup(peer_id, ChannelKind::Bluetooth)
                .is_some_and(|setup| setup.accepted_peer_request)
        {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Dials `address` and sends this device's hello (ADR-0008 D1).
    /// `Some(())` if the device there is `peer_id` and acknowledged it --
    /// which it does only while it is running its own setup search for us
    /// (D2).
    async fn send_request(&self, state: &DeviceConnectionState, address: &str, peer_id: &str) -> Option<()> {
        let mut link = self.connect_session(state, peer_id, address, PEER_ALPN).await.ok()?;
        send_frame(
            link.as_mut(),
            &PeerFrame::AddChannel {
                device_id: state.identity.device_id.clone(),
            },
        )
        .await
        .ok()?;
        match recv_frame(link.as_mut()).await {
            Some(Ok(PeerFrame::AddChannelAccept { device_id })) if device_id == peer_id => Some(()),
            _ => None,
        }
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
    #[test]
    fn forgetting_delivery_misses_makes_the_next_search_due_now() {
        let schedule = DeliverySchedule::default();
        let scanner = Scanner::default();
        let peer = "peer-forget-delivery-misses";
        schedule.note_delivery_missed(peer, &SpaceSync::default());
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
