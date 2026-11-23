//! The Bluetooth channel uses one shared managed `PeerLink`.
//! Plain identify frames and iroh share that owner and its scoped channels.
//! Independent incoming attempts reach Fini's existing pairing and session checks.
//! The iroh endpoint has no IP transports or relay.
//!
//! Native lifecycle and crossed BLE attempts belong to the core;
//! application eligibility, trust, channel switches and delivery belong to Fini.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
// The driver of a desktop system: BlueZ on Linux, WinRT on Windows.
use ble_gatt::datagram::DatagramConfig;
#[cfg(target_os = "linux")]
use ble_gatt::drivers::linux::LinuxDriver as DesktopDriver;
#[cfg(target_os = "windows")]
use ble_gatt::drivers::windows::WindowsDriver as DesktopDriver;
use ble_gatt::{
    Adapter, CharacteristicUuid, PeerAddress, PeerChannel, PeerLink, PeerLinkConfig, ServiceUuid,
};
use uuid::Uuid;

use crate::models::{Event, Identity};
use crate::repositories::{ChannelRepository, DeviceRepository, Repository};
use crate::services::channel_service::iroh_link::{IrohDataLink, PEER_ALPN, SYNC_ALPN};
use crate::services::channel_service::{
    ChannelKind, ChannelProblem, ChannelService, DataLink, LinkHandlerSlot, LinkHandlers,
};
use crate::services::db::DbPool;
use crate::services::event_bus_service::EventBus;
use crate::services::space_sync_service::rejected_by_peer;

#[path = "bluetooth-channel/adapter.rs"]
mod adapter;
#[path = "bluetooth-channel/advertiser.rs"]
mod advertiser;
#[path = "bluetooth-channel/data-link.rs"]
mod data_link;
#[path = "bluetooth-channel/endpoint.rs"]
mod endpoint;
#[path = "bluetooth-channel/scanner.rs"]
mod scanner;

pub use adapter::BluetoothAdapter;
pub use data_link::*;

pub use advertiser::Advertiser;
pub use scanner::AvailableDevice;
pub use scanner::Scanner;

use scanner::Purpose;

/// The Bluetooth channel. It owns the scan and what the scan heard, the
/// advertisement, the delivery retry schedule and the iroh endpoint over
/// Bluetooth.
pub struct BluetoothChannelService {
    identity: Identity,
    pool: DbPool,
    bus: Arc<EventBus>,
    /// This object, for the tasks it starts.
    me: Weak<BluetoothChannelService>,
    handlers: LinkHandlerSlot,
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
    /// One managed BLE owner for plain frames and iroh.
    peer_link: tokio::sync::OnceCell<Arc<PeerLink>>,
    /// Stop work before draining the owners that already exist.
    stopping: tokio::sync::watch::Sender<bool>,
    initialization: tokio::sync::RwLock<()>,
    admission: std::sync::Mutex<()>,
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    shutdown_result: tokio::sync::OnceCell<Result<(), String>>,
    /// Scoped pairing uses until each first session acquires the channel.
    pairing_links:
        std::sync::Mutex<std::collections::HashMap<String, (Uuid, iroh::EndpointId, PeerChannel)>>,
    /// When this device last rejected each paired device's session because
    /// the channel was off here (`note_refused`).
    refused_at: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
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

/// After a delivery search heard the peer but no connect to it finished,
/// the next search waits this long. Short: each connect is bounded by its
/// own timeout, so the retries do not run in a tight loop.
const FAILED_CONNECT_WAIT_TIME: Duration = Duration::from_secs(2);

/// How long the device with the higher device id waits for the peer's session
/// before it connects itself (`answers_first`). A connect and its handshake
/// took 1-5 s on hardware.
const ANSWER_FIRST_WAIT: Duration = Duration::from_secs(8);

/// Whether an exchange of the device that answers first waits
/// `ANSWER_FIRST_WAIT` before each connect (`peer_connected_first`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AnswerFirst {
    /// Wait for the peer's session first: the usual exchange.
    Wait,
    /// Connect at once: the peer is in its rejection wait and does not
    /// connect (`connect_once_after_switch_on`).
    Skip,
}

/// After a failed advertisement update, `run_server` tries again after this
/// wait, doubled after each failure in a row, up to
/// `ADVERTISING_RETRY_MAX_WAIT`.
const ADVERTISING_RETRY_WAIT: Duration = Duration::from_secs(1);
const ADVERTISING_RETRY_MAX_WAIT: Duration = Duration::from_secs(30);

/// The wait before the next advertisement update after `failed_updates`
/// failed updates in a row: 1, 2, 4, 8 and 16 s, then always 30 s.
fn advertising_retry_wait(failed_updates: u32) -> Duration {
    let doublings = failed_updates.saturating_sub(1).min(5);
    ADVERTISING_RETRY_WAIT
        .saturating_mul(1 << doublings)
        .min(ADVERTISING_RETRY_MAX_WAIT)
}

/// Per paired device: how many delivery searches in a row found nothing,
/// and when the next one is due; and until when a rejection stops the next
/// connection. In memory only, as ADR-0008 decided: a started app tries at
/// once.
pub struct DeliverySchedule {
    bus: Arc<EventBus>,
    schedule: std::sync::Mutex<std::collections::HashMap<String, (usize, std::time::Instant)>>,
    rejected_until: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl DeliverySchedule {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self {
            bus,
            schedule: Default::default(),
            rejected_until: Default::default(),
        }
    }

    /// Tells the bus once `after` has passed: a search or a connection to
    /// the peer is due again (ADR-0008 D12).
    fn publish_due_after(&self, peer_id: &str, after: Duration) {
        let bus = self.bus.clone();
        let event = Event::ConnectChanged {
            peer_device_id: peer_id.to_string(),
            kind: ChannelKind::Bluetooth,
        };
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(after).await;
            bus.publish(event);
        });
    }

    /// Whether a delivery search for this device is due. A device that
    /// rejected a session recently is not. A device heard recently is
    /// otherwise always due: it is reached by one connection.
    pub fn delivery_due(&self, scanner: &Scanner, peer_id: &str) -> bool {
        let rejected = self.rejected_until.lock().is_ok_and(|rejected| {
            rejected
                .get(peer_id)
                .is_some_and(|until| std::time::Instant::now() < *until)
        });
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

    /// The peer was heard, but no connect to it finished: the next search
    /// is due after `FAILED_CONNECT_WAIT_TIME`, and it is not a miss.
    pub fn retry_after_failed_connect(&self, peer_id: &str) {
        self.publish_due_after(peer_id, FAILED_CONNECT_WAIT_TIME);
    }

    /// The peer rejected a session: the next connection waits
    /// `REJECTED_WAIT_TIME`.
    pub fn record_rejected(&self, peer_id: &str) {
        if let Ok(mut rejected) = self.rejected_until.lock() {
            rejected.insert(
                peer_id.to_string(),
                std::time::Instant::now() + REJECTED_WAIT_TIME,
            );
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
        CharacteristicUuid(
            Uuid::parse_str(FINI_BLE_CHARACTERISTIC_UUID).expect("valid UUID literal"),
        ),
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
/// fingerprint is how a scanner tells which peer it has found.
///
/// The record travels in ble-gatt's fast advertisement
/// (`DatagramConfig::fast_advertisement_company`, ble-gatt ADR-0012 §2):
/// the endpoint, a hash of the readable metadata and these five bytes, under
/// `0xFFFF`, in the Android scan response. A scanner gets the five bytes
/// back under `0xFFFF` and lists the phone with no connection. The short
/// name no longer fits there; it comes from one read later. Windows sends
/// the fast advertisement in a publisher advertisement of its own, from
/// another address than its GATT server; the scanner joins the two after
/// one read. Linux cannot send it; scanners read its record.
/// A build before this change reads the fast advertisement as a wrong
/// record (accepted: the two pull requests ship together).
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

    fn pool(&self) -> &DbPool {
        &self.pool
    }

    fn set_link_handlers(&self, handlers: LinkHandlers) {
        self.handlers.set(handlers);
    }

    fn set_add_device_mode(&self, on: bool) {
        if self.stopping() {
            return;
        }
        self.advertiser.set_add_mode(on);
        self.scanner.set_add_device_mode(on);
        tokio::task::block_in_place(|| self.refresh_advertising(&mut self.pool.connection()));
    }

    /// Linux, Windows and Android: the only builds with a Bluetooth channel.
    fn available(&self) -> bool {
        true
    }

    fn usable(&self) -> bool {
        !self.stopping() && !BluetoothAdapter::this_device().is_unavailable()
    }

    async fn probe(&self) -> bool {
        tokio::select! {
            biased;
            () = self.wait_until_stopping() => false,
            usable = BluetoothAdapter::this_device().probe(&self.scanner) => usable,
        }
    }

    fn start_serving(self: Arc<Self>) {
        // Desktop only (Linux and Windows), deliberately. Android starts the
        // same loop from `keep_serving`, once its Activity context exists --
        // starting it here as well gave Android two accept loops racing each
        // other.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        tauri::async_runtime::spawn(async move { self.run_server().await });
    }

    fn keep_serving(self: Arc<Self>) {
        if self.stopping() {
            return;
        }
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
        tokio::task::block_in_place(|| self.refresh_advertising(&mut self.pool.connection()));
        // Hold a session with every paired device that has Bluetooth on
        // (2026-10-13: green is a live connection). `start_exchange` skips
        // a device that has one, or whose delivery search is not due.
        let peers = tokio::task::block_in_place(|| {
            ChannelRepository::new(&mut self.pool.connection())
                .peers_with_channel_enabled(ChannelKind::Bluetooth)
        });
        for peer in peers {
            self.start_exchange(&peer);
        }
    }

    fn forget_failures(&self, peer_device_id: &str) {
        self.delivery.forget_delivery_misses(peer_device_id);
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
    pub fn new(identity: Identity, pool: DbPool, bus: Arc<EventBus>) -> Arc<Self> {
        BluetoothAdapter::this_device().set_bus(bus.clone());
        Arc::new_cyclic(|me| Self {
            scanner: Arc::new(Scanner::new(bus.clone(), me.clone())),
            advertiser: Arc::new(Advertiser::new(&identity)),
            delivery: DeliverySchedule::new(bus.clone()),
            endpoint: tokio::sync::OnceCell::new(),
            peer_link: tokio::sync::OnceCell::new(),
            stopping: tokio::sync::watch::channel(false).0,
            initialization: tokio::sync::RwLock::new(()),
            admission: std::sync::Mutex::new(()),
            #[cfg(any(target_os = "linux", target_os = "windows", test))]
            shutdown_result: tokio::sync::OnceCell::new(),
            pairing_links: std::sync::Mutex::default(),

            refused_at: std::sync::Mutex::default(),
            identity,
            pool,
            bus,
            me: me.clone(),
            handlers: LinkHandlerSlot::default(),
        })
    }

    /// This object as an `Arc`, for a task it starts.
    fn arc(&self) -> Arc<Self> {
        self.me
            .upgrade()
            .expect("the Bluetooth channel lives as long as its tasks")
    }

    fn stopping(&self) -> bool {
        *self.stopping.borrow()
    }

    async fn wait_until_stopping(&self) {
        let mut stopping = self.stopping.subscribe();
        let _ = stopping.wait_for(|stopping| *stopping).await;
    }

    /// True owner shutdown only. Each drain has a bound; errors remain in the result.
    /// Android does not use it: the Activity does not own the channel.
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    pub async fn shutdown(&self) -> Result<(), String> {
        {
            let _admission = self.admission.lock().unwrap_or_else(|err| err.into_inner());
            self.stopping.send_replace(true);
        }
        self.scanner.stop();
        self.shutdown_result
            .get_or_init(|| async {
                let mut errors = Vec::new();
                let initialization =
                    match tokio::time::timeout(Duration::from_secs(2), self.initialization.write())
                        .await
                    {
                        Ok(guard) => Some(guard),
                        Err(err) => {
                            errors.push(format!("Bluetooth initialization drain timed out: {err}"));
                            None
                        }
                    };
                if let Some((endpoint, _)) = self.endpoint.get() {
                    if let Err(err) =
                        tokio::time::timeout(Duration::from_secs(2), endpoint.close()).await
                    {
                        errors.push(format!("Bluetooth endpoint close timed out: {err}"));
                    }
                }
                let held = self
                    .pairing_links
                    .lock()
                    .map(|mut links| {
                        links
                            .drain()
                            .map(|(_, (_, _, channel))| channel)
                            .collect::<Vec<_>>()
                    })
                    .map_err(|err| err.to_string());
                match held {
                    Ok(held) => {
                        let drain = async {
                            let mut releases = futures_util::stream::FuturesUnordered::new();
                            for channel in held {
                                releases.push(async move { channel.release().await });
                            }
                            use futures_util::StreamExt;
                            while let Some(result) = releases.next().await {
                                if let Err(err) = result {
                                    errors
                                        .push(format!("Bluetooth held use release failed: {err}"));
                                }
                            }
                        };
                        if let Err(err) = tokio::time::timeout(Duration::from_secs(2), drain).await
                        {
                            errors.push(format!("Bluetooth held use drain timed out: {err}"));
                        }
                    }
                    Err(err) => errors.push(format!("Bluetooth held use lock failed: {err}")),
                }
                // Always attempt managed close, even when an earlier drain failed.
                if let Some(link) = self.peer_link.get() {
                    match tokio::time::timeout(Duration::from_secs(6), link.close()).await {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => errors.push(format!("Bluetooth owner close failed: {err}")),
                        Err(err) => errors.push(format!("Bluetooth owner close timed out: {err}")),
                    }
                }
                drop(initialization);
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("; "))
                }
            })
            .await
            .clone()
    }

    /// Whether a session with the peer runs on Bluetooth now.
    pub fn has_session(&self, peer_id: &str) -> bool {
        self.handlers.has_session(peer_id, ChannelKind::Bluetooth)
    }

    /// The iroh key pinned for the pair, if any.
    fn pinned_key(&self, peer_id: &str) -> Option<String> {
        tokio::task::block_in_place(|| {
            DeviceRepository::new(&mut self.pool.connection()).pinned_key(peer_id)
        })
    }

    /// Follow advertisement state. Incoming requests have their own task.
    async fn run_server(&self) {
        if self.stopping() || !self.advertiser.claim_peripheral_role() {
            return;
        }
        let mut wanted = self.advertiser.watch_wanted();
        let mut add_mode = self.advertiser.watch_add_mode();
        let mut events = self.bus.subscribe();
        let mut link = None;
        let mut uncertain = false;
        let mut failed_updates: u32 = 0;
        loop {
            let visible = *wanted.borrow_and_update();
            let _ = *add_mode.borrow_and_update();
            if self.stopping() {
                return;
            }
            if visible || link.is_some() {
                let apply = async {
                    if link.is_none() {
                        // No managed owner means no advertisement: try again
                        // like after a failed update.
                        match self.peer_link().await {
                            Ok(shared) => link = Some(shared),
                            Err(err) => {
                                if !self.stopping() {
                                    uncertain = true;
                                    BluetoothAdapter::this_device().note_unreachable();
                                }
                                return Err(err);
                            }
                        }
                    }
                    let shared = link.as_ref().expect("the managed owner exists");
                    self.advertiser
                        .update(shared, visible, &mut uncertain)
                        .await
                };
                tokio::select! {
                    biased;
                    () = self.wait_until_stopping() => return,
                    result = apply => if let Err(err) = result {
                        log::warn!("[transport][ble] advertisement update failed: {err}");
                    },
                }
                failed_updates = if uncertain {
                    failed_updates.saturating_add(1)
                } else {
                    0
                };
            } else {
                // Not wanted and no owner: nothing to try again.
                uncertain = false;
                failed_updates = 0;
            }
            // A failed update leaves the advertisement hidden or old: try it
            // again after a wait that grows, or when the adapter changes.
            let retry = tokio::time::sleep(advertising_retry_wait(failed_updates));
            tokio::pin!(retry);
            loop {
                tokio::select! {
                    biased;
                    () = self.wait_until_stopping() => return,
                    result = wanted.changed() => {
                        if result.is_err() { return; }
                        break;
                    },
                    result = add_mode.changed() => {
                        if result.is_err() { return; }
                        break;
                    },
                    () = &mut retry, if uncertain => break,
                    event = events.recv(), if (visible && link.is_none()) || uncertain => {
                        match event {
                            Ok(Event::AdapterChanged) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                if self.peer_link.get().is_some() || !BluetoothAdapter::this_device().is_unavailable() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                            _ => {}
                        }
                    },
                }
            }
        }
    }

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

    /// The shared managed owner. Claim incoming requests before any advertisement.
    fn peer_link(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Arc<PeerLink>, String>> + Send + '_>,
    > {
        Box::pin(async move {
            let _initialization = self.initialization.write().await;
            if self.stopping() {
                return Err("Bluetooth is stopping".to_string());
            }
            if let Some(link) = self.peer_link.get() {
                return Ok(link.clone());
            }
            use futures_util::StreamExt;

            let adapter = tokio::select! {
                biased;
                () = self.wait_until_stopping() => return Err("Bluetooth is stopping".to_string()),
                adapter = BluetoothAdapter::this_device().adapter() => adapter?,
            };
            let _admission = self.admission.lock().map_err(|err| err.to_string())?;
            if self.stopping() {
                return Err("Bluetooth is stopping".to_string());
            }
            let link = PeerLink::with_driver(
                adapter.driver(),
                PeerLinkConfig::new(self.advertiser.advertised_config()),
            );
            let mut incoming = link.incoming().map_err(|err| err.to_string())?;
            self.peer_link
                .set(link.clone())
                .map_err(|_| "Bluetooth owner already initialized".to_string())?;
            let owner = self.me.clone();
            tokio::spawn(async move {
                loop {
                    let Some(owner) = owner.upgrade() else { return };
                    let request = tokio::select! {
                        biased;
                        () = owner.wait_until_stopping() => return,
                        request = incoming.next() => match request { Some(request) => request, None => return },
                    };
                    // Admission is independent of any outgoing attempt.
                    tokio::spawn(async move {
                        if owner.stopping()
                            || (!owner.advertiser.in_add_mode() && !owner.advertiser.wanted())
                        {
                            if let Err(err) = request.reject().await {
                                log::warn!("[transport][ble] request rejection failed: {err}");
                            }
                            return;
                        }
                        let accepted = tokio::select! {
                            biased;
                            () = owner.wait_until_stopping() => return,
                            accepted = request.accept() => accepted,
                        };
                        match accepted {
                            Ok(channel) => {
                                let _step = owner.scanner.begin_pairing_step();
                                if let Some(handled) = owner.route_inbound(channel).await {
                                    let _ = handled.await;
                                }
                            }
                            Err(err) => {
                                log::info!("[transport][ble] incoming attempt ended: {err}");
                            }
                        }
                    });
                }
            });
            // The shared owner is ready, also when the scanner initialized it.
            self.bus.publish(Event::AdapterChanged);
            Ok::<_, String>(link)
        })
    }

    /// Start an exchange with this peer over Bluetooth unless one is running
    /// or being attempted (ADR-0008 D10). Called when there is work for the
    /// peer.
    pub fn start_exchange(&self, peer_id: &str) {
        self.start_exchange_with(peer_id, AnswerFirst::Wait);
    }

    fn start_exchange_with(&self, peer_id: &str, answer_first: AnswerFirst) {
        if self.stopping()
            || self.has_session(peer_id)
            || !self.delivery.delivery_due(&self.scanner, peer_id)
        {
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
            tokio::select! {
                biased;
                () = bluetooth.wait_until_stopping() => {},
                () = bluetooth.exchange_with(&peer_id, answer_first) => {},
            }
            bluetooth.scanner.end_exchange(&peer_id);
        });
    }

    /// Connect after switch-on when the peer can still be in its rejection wait.
    /// This connect does not wait `ANSWER_FIRST_WAIT`: the peer does not connect
    /// during its `REJECTED_WAIT_TIME`, so a wait here only made green later.
    pub fn connect_once_after_switch_on(&self, peer_id: &str) {
        let refused_recently = self
            .refused_at
            .lock()
            .ok()
            .and_then(|mut refused| refused.remove(peer_id))
            .is_some_and(|at| at.elapsed() < REJECTED_WAIT_TIME);
        if !refused_recently {
            return;
        }
        log::info!(
            "[transport][ble] {peer_id} was rejected a short time ago; connecting to it once"
        );
        self.delivery.forget_delivery_misses(peer_id);
        self.start_exchange_with(peer_id, AnswerFirst::Skip);
    }

    /// This device rejected `peer_id`'s session: the channel is off here.
    pub fn note_refused(&self, peer_id: &str) {
        if let Ok(mut refused) = self.refused_at.lock() {
            refused.insert(peer_id.to_string(), std::time::Instant::now());
        }
    }

    /// One delivery (ADR-0008 D8, D12): a present peer is one dial to where
    /// it was last heard; otherwise a delivery search of up to
    /// `SEARCH_WINDOW` connects the moment the peer is heard. Then
    /// authenticate and run the exchange to its idle end.
    async fn exchange_with(&self, peer_id: &str, answer_first: AnswerFirst) {
        if !self.is_on(peer_id) {
            return;
        }
        // Just paired over Bluetooth: the pairing's BLE link is still up, so
        // the session goes over it, with no search and no new connect.
        if let Some(channel) = self.remove_pairing_link(peer_id) {
            let address = channel.peer().0;
            let guard = self.scanner.acquire_connect().await;
            let connected = self.connect_for_session(peer_id, &address).await;
            if let Err(err) = channel.release().await {
                log::warn!("[transport][ble] pairing channel release failed: {err}");
            }
            match connected {
                Ok(link) => {
                    drop(guard);
                    log::info!(
                        "[transport][ble] first session with {peer_id} over the pairing link"
                    );
                    return self.run_exchange(peer_id, link, &address).await;
                }
                Err(err) => {
                    log::info!("[transport][ble] the pairing link to {address} is gone: {err}");
                }
            }
        }
        let deadline = tokio::time::Instant::now() + SEARCH_WINDOW;
        let mut last_error = None;

        if let Some(address) = self.scanner.last_heard_address(peer_id) {
            if self.peer_connected_first(peer_id, answer_first).await {
                return;
            }
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
                // A search that went on after an unpair kept this exchange,
                // so the first session of the next pair did not start.
                tokio::select! {
                    biased;
                    found = self.scanner.find(peer_id, Purpose::Delivery, remaining) => found,
                    () = self.wait_until_off(peer_id) => {
                        log::info!("[transport][ble] stopped the delivery search for {peer_id}: it has no Bluetooth channel any more");
                        return;
                    }
                }
            };
            let Some(found) = found else {
                match last_error {
                    // The peer was heard, but no connect to it finished. That
                    // is not an absent peer: waiting `DELIVERY_RETRY` after it
                    // kept the devices grey for 60 s more. A refusal by the
                    // peer is not here: `run_exchange` records it.
                    Some(err) => {
                        log::info!("[transport][ble] no connect to {peer_id} finished: {err}");
                        self.delivery.retry_after_failed_connect(peer_id);
                    }
                    None => {
                        log::info!("[transport][ble] {peer_id} not heard within the search window");
                        self.delivery.note_delivery_missed(peer_id);
                    }
                }
                return;
            };
            // The peer's session can arrive while this search runs: it carries
            // the work, and a dial now would only cross it.
            if !self.is_on(peer_id)
                || self.has_session(peer_id)
                || self.peer_connected_first(peer_id, answer_first).await
            {
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

    /// Crossed session connects: when both paired devices have work, both
    /// start an exchange and connect at once. On hardware the two links then
    /// ended each other: ble-gatt's arbitration kept one managed attempt, but
    /// the iroh path of the other side moved to the losing link and its
    /// handshake was aborted, so neither side turned green
    /// (peer-sync-over-ble). Only one side connects first: the device with the
    /// smaller device id, whose connection the session accept keeps on a
    /// crossing anyway (`space_sync_service`, "the connection from the smaller
    /// device id is kept"). The other answers first: it waits
    /// `ANSWER_FIRST_WAIT` for the session that the peer's connect opens, and
    /// connects itself only when none came (the peer has no work, or cannot
    /// hear this device). Either side can still start a session. The connect
    /// after switch-on (`connect_once_after_switch_on`) does not wait.
    fn answers_first(&self, peer_id: &str) -> bool {
        self.identity.device_id.as_str() > peer_id
    }

    /// Before each connect of the device that answers first: waits up to
    /// `ANSWER_FIRST_WAIT` for the session the peer opens. True when one runs,
    /// so this device does not connect. The wait is per connect, not once per
    /// exchange: on hardware the phone's exchange started 12 s before the PC
    /// app was up, and a wait at its start had ended when the PC connected.
    /// `AnswerFirst::Skip` connects at once.
    async fn peer_connected_first(&self, peer_id: &str, answer_first: AnswerFirst) -> bool {
        if answer_first == AnswerFirst::Skip || !self.answers_first(peer_id) {
            return false;
        }
        let connected = self.wait_for_peer_session(peer_id).await;
        if connected {
            log::info!("[transport][ble] {peer_id} connected first; no connect from here");
        }
        connected
    }

    /// Waits up to `ANSWER_FIRST_WAIT` for a session with `peer_id` that the
    /// peer opened. True when one runs. Wakes on each event of the bus, for
    /// example `SessionChanged`, which SpaceSync sends after it records the
    /// session.
    async fn wait_for_peer_session(&self, peer_id: &str) -> bool {
        let mut events = self.bus.subscribe();
        let wait = async {
            while !self.has_session(peer_id) {
                // Any event, or an overrun, reads the sessions again.
                if let Err(tokio::sync::broadcast::error::RecvError::Closed) = events.recv().await {
                    std::future::pending::<()>().await;
                }
            }
        };
        tokio::select! {
            biased;
            () = self.wait_until_stopping() => false,
            () = self.wait_until_off(peer_id) => false,
            waited = tokio::time::timeout(ANSWER_FIRST_WAIT, wait) => waited.is_ok(),
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
    async fn connect_for_session(
        &self,
        peer_id: &str,
        address: &str,
    ) -> Result<Box<dyn DataLink>, String> {
        let _connect_mark = self.scanner.begin_session_connect(peer_id);
        let connect = tokio::time::timeout(
            CONNECT_CANDIDATE_TIMEOUT,
            self.connect_session(peer_id, address, SYNC_ALPN),
        );
        let attempt = tokio::select! {
            biased;
            () = self.wait_until_stopping() => return Err("Bluetooth is stopping".to_string()),
            attempt = connect => attempt,
            () = self.wait_until_off(peer_id) => {
                log::info!("[transport][ble] stopped the connect to {address}: {peer_id} has no Bluetooth channel any more");
                // Shared carrier users must stay valid.
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
                log::info!(
                    "[transport][ble] candidate {address} did not finish the connect in time"
                );
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
            ChannelRepository::new(&mut self.pool.connection()).note_address(
                peer_id,
                ChannelKind::Bluetooth,
                address,
            );
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
        // Endpoint hooks release only the ended iroh user.
    }

    /// Works out again whether to advertise, from the stored channels and
    /// the setups open now. Cheap; called whenever one of these can change.
    /// The caller gives its connection (docs/db.md, rule 2).
    pub fn refresh_advertising(&self, conn: &mut diesel::SqliteConnection) {
        let any_channel_on = !ChannelRepository::new(conn)
            .peers_with_channel_enabled(ChannelKind::Bluetooth)
            .is_empty();
        let advertiser = &self.advertiser;
        let wanted = any_channel_on || advertiser.in_add_mode();
        advertiser.set_wanted(wanted);
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
            .map(|misses| {
                next_delivery_search(misses, now)
                    .duration_since(now)
                    .as_secs()
            })
            .collect();
        assert_eq!(waits, vec![60, 300, 900, 900, 900]);
    }

    /// Switching Bluetooth on tries again at once, whatever retry delay
    /// earlier misses left behind.
    #[tokio::test]
    async fn forgetting_delivery_misses_makes_the_next_search_due_now() {
        let schedule = DeliverySchedule::new(Arc::new(EventBus::default()));
        let scanner = Scanner::default();
        let peer = "peer-forget-delivery-misses";
        schedule.note_delivery_missed(peer);
        assert!(
            !schedule.delivery_due(&scanner, peer),
            "a miss delays the next search"
        );
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
            assert_eq!(
                fingerprint_of(id),
                reference_fnv1a(id),
                "mismatch for {id:?}"
            );
        }
        assert_ne!(fingerprint_of("peer-a"), fingerprint_of("peer-b"));
    }

    const PEER: &str = "peer-a";
    const PEER_ADDRESS: &str = "AA:00:00:00:00:20";
    /// Larger than `PEER`: this device answers first (`answers_first`).
    const LARGER_DEVICE_ID: &str = "zz-this-device";

    /// Sessions that the test starts by hand.
    #[derive(Default)]
    struct TestSessions(std::sync::atomic::AtomicBool);

    #[async_trait]
    impl crate::services::channel_service::SessionHandler for TestSessions {
        fn has_session(&self, _peer_device_id: &str, _kind: ChannelKind) -> bool {
            self.0.load(Ordering::SeqCst)
        }

        async fn hold<'a>(
            &'a self,
            link: Box<dyn DataLink>,
            _peer_device_id: &'a str,
            _kind: ChannelKind,
            _on_held: Box<dyn FnOnce() + Send + 'a>,
        ) -> Result<Option<u32>, Box<dyn DataLink>> {
            Err(link)
        }

        async fn accept(&self, _link: Box<dyn DataLink>) {}
    }

    struct NoPeerLinks;

    #[async_trait]
    impl crate::services::channel_service::PeerLinkHandler for NoPeerLinks {
        async fn accept(&self, _link: Box<dyn DataLink>) {}
    }

    struct Fixture {
        service: Arc<BluetoothChannelService>,
        sessions: Arc<TestSessions>,
        db_path: std::path::PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.db_path);
        }
    }

    /// A Bluetooth channel with `device_id` on a temporary database, paired
    /// with `PEER`, Bluetooth on. The pair has no pinned key, so a session
    /// connect to `PEER` fails at once and marks its address as failed: the
    /// mark shows that a connect ran.
    fn fixture(label: &str, device_id: &str) -> Fixture {
        let db_path =
            std::env::temp_dir().join(format!("fini-test-bluetooth-{label}-{}.db", Uuid::new_v4()));
        pair_with_peer(&mut crate::services::db::open_db_at_path(&db_path));
        let secret_key = iroh::SecretKey::generate();
        let identity = Identity {
            device_id: device_id.to_string(),
            hostname: "host".to_string(),
            endpoint_id: secret_key.public().to_string(),
            secret_key,
        };
        let service = BluetoothChannelService::new(
            identity,
            DbPool::open_for_test(&db_path),
            Arc::new(EventBus::default()),
        );
        let sessions = Arc::new(TestSessions::default());
        service.handlers.set(LinkHandlers {
            peer: Arc::new(NoPeerLinks),
            session: sessions.clone(),
        });
        Fixture {
            service,
            sessions,
            db_path,
        }
    }

    /// Pairs with `PEER`, with its Bluetooth channel on.
    fn pair_with_peer(conn: &mut diesel::SqliteConnection) {
        use crate::schema::paired_devices;
        use diesel::prelude::*;

        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq(PEER),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(conn)
            .unwrap();
        ChannelRepository::new(conn)
            .configure(PEER, ChannelKind::Bluetooth, true, None)
            .unwrap();
    }

    async fn wait_until(what: &str, within: Duration, done: impl Fn() -> bool) {
        let wait = async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(within, wait)
            .await
            .unwrap_or_else(|_| panic!("{what} within {within:?}"));
    }

    /// The device with the larger id does not connect while it waits for the
    /// peer's session, and when that session starts, its exchange ends with
    /// no connect, before `ANSWER_FIRST_WAIT` has passed.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_device_that_answers_first_waits_for_the_peer_session() {
        let fixture = fixture("answer-first", LARGER_DEVICE_ID);
        let service = &fixture.service;
        service.scanner.record_paired_heard(PEER, PEER_ADDRESS);

        service.start_exchange(PEER);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !service.scanner.connect_recently_failed(PEER_ADDRESS),
            "no connect while it waits for the peer's session"
        );

        fixture.sessions.0.store(true, Ordering::SeqCst);
        service.bus.publish(Event::SessionChanged {
            peer_device_id: PEER.to_string(),
            kind: ChannelKind::Bluetooth,
            established: true,
        });
        wait_until("the exchange ends", Duration::from_secs(2), || {
            let ended = service.scanner.begin_exchange(PEER);
            if ended {
                service.scanner.end_exchange(PEER);
            }
            ended
        })
        .await;
        assert!(
            !service.scanner.connect_recently_failed(PEER_ADDRESS),
            "the peer's session carries the work: no connect from here"
        );
        service.shutdown().await.unwrap();
    }

    /// After switch-on, the connect to a peer that this device rejected a
    /// short time ago starts at once, also on the device with the larger id:
    /// the peer is in its rejection wait and does not connect.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_connect_after_switch_on_does_not_wait_for_the_peer() {
        let fixture = fixture("connect-once", LARGER_DEVICE_ID);
        let service = &fixture.service;
        service.scanner.record_paired_heard(PEER, PEER_ADDRESS);
        service.note_refused(PEER);

        service.connect_once_after_switch_on(PEER);
        wait_until(
            "a connect to the peer starts",
            Duration::from_secs(2),
            || service.scanner.connect_recently_failed(PEER_ADDRESS),
        )
        .await;
        service.shutdown().await.unwrap();
    }

    /// An unpair stops a delivery search that runs: the exchange ends at
    /// once, not at the end of `SEARCH_WINDOW`. Then the exchange of a new
    /// pair with the same device starts and connects.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unpair_stops_the_delivery_search() {
        // Smaller than `PEER`: this device connects first, with no wait.
        let fixture = fixture("unpair-search", "aa-this-device");
        let service = &fixture.service;

        // Not heard yet: the exchange starts a delivery search.
        service.start_exchange(PEER);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !service.scanner.begin_exchange(PEER),
            "the exchange runs its delivery search"
        );

        DeviceRepository::new(&mut service.pool.connection())
            .remove(PEER)
            .unwrap();
        wait_until("the exchange ends", Duration::from_secs(3), || {
            let ended = service.scanner.begin_exchange(PEER);
            if ended {
                service.scanner.end_exchange(PEER);
            }
            ended
        })
        .await;

        pair_with_peer(&mut service.pool.connection());
        service.scanner.record_paired_heard(PEER, PEER_ADDRESS);
        service.start_exchange(PEER);
        wait_until(
            "the exchange of the new pair connects",
            Duration::from_secs(3),
            || service.scanner.connect_recently_failed(PEER_ADDRESS),
        )
        .await;
        service.shutdown().await.unwrap();
    }

    /// A mock radio that refuses the first start of the advertisement.
    struct FirstAdvertiseFails {
        driver: Arc<dyn ble_gatt::Driver>,
        /// Calls of `advertise`, the refused one too.
        advertise_calls: std::sync::atomic::AtomicUsize,
        /// Starts of the advertisement that the radio accepted.
        started: std::sync::atomic::AtomicUsize,
    }

    impl FirstAdvertiseFails {
        fn new() -> Self {
            use ble_gatt::drivers::mock::{MockDriver, MockNetwork};
            Self {
                driver: Arc::new(MockDriver::new(
                    PeerAddress("AA:00:00:00:00:30".to_string()),
                    MockNetwork::new(),
                    ble_gatt::CapabilityReport {
                        central: true,
                        peripheral: true,
                    },
                )),
                advertise_calls: Default::default(),
                started: Default::default(),
            }
        }
    }

    #[async_trait]
    impl ble_gatt::Driver for FirstAdvertiseFails {
        async fn capabilities(&self) -> ble_gatt::CapabilityReport {
            self.driver.capabilities().await
        }

        async fn scan(
            &self,
            service: ServiceUuid,
        ) -> ble_gatt::Result<ble_gatt::BoxStream<ble_gatt::Result<ble_gatt::DiscoveredPeer>>>
        {
            self.driver.scan(service).await
        }

        async fn connect(
            &self,
            peer: &PeerAddress,
        ) -> ble_gatt::Result<Box<dyn ble_gatt::GattConnection>> {
            self.driver.connect(peer).await
        }

        async fn advertise(&self, service: ble_gatt::GattServiceSpec) -> ble_gatt::Result<()> {
            if self.advertise_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ble_gatt::BleError::AdapterUnavailable(
                    "the test radio refuses the first advertisement".into(),
                ));
            }
            self.driver.advertise(service).await?;
            self.started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn update_advertisement(
            &self,
            service: ble_gatt::GattServiceSpec,
        ) -> ble_gatt::Result<()> {
            self.driver.update_advertisement(service).await
        }

        fn served_service(&self) -> Option<ble_gatt::GattServiceSpec> {
            self.driver.served_service()
        }

        async fn set_characteristic_value(
            &self,
            characteristic: CharacteristicUuid,
            value: Vec<u8>,
        ) -> ble_gatt::Result<()> {
            self.driver
                .set_characteristic_value(characteristic, value)
                .await
        }

        async fn set_advertising_enabled(&self, enabled: bool) -> ble_gatt::Result<()> {
            self.driver.set_advertising_enabled(enabled).await
        }

        async fn stop_advertising(&self) -> ble_gatt::Result<()> {
            self.driver.stop_advertising().await
        }

        async fn close(&self) -> ble_gatt::Result<()> {
            self.driver.close().await
        }

        async fn notify(
            &self,
            characteristic: CharacteristicUuid,
            value: Vec<u8>,
        ) -> ble_gatt::Result<()> {
            self.driver.notify(characteristic, value).await
        }

        async fn notify_peer(
            &self,
            peer: &PeerAddress,
            session: Option<u64>,
            characteristic: CharacteristicUuid,
            value: Vec<u8>,
        ) -> ble_gatt::Result<()> {
            self.driver
                .notify_peer(peer, session, characteristic, value)
                .await
        }

        async fn disconnect_peer(
            &self,
            peer: &PeerAddress,
            session: Option<u64>,
        ) -> ble_gatt::Result<()> {
            self.driver.disconnect_peer(peer, session).await
        }

        async fn notify_max_len(&self, peer: &PeerAddress, session: Option<u64>) -> Option<usize> {
            self.driver.notify_max_len(peer, session).await
        }

        fn events(&self) -> ble_gatt::BoxStream<ble_gatt::GattEvent> {
            self.driver.events()
        }

        fn lifecycle_events(&self) -> ble_gatt::BoxStream<ble_gatt::GattEvent> {
            self.driver.lifecycle_events()
        }

        async fn radio_status(&self) -> ble_gatt::RadioStatus {
            self.driver.radio_status().await
        }
    }

    /// A failed advertisement update is tried again with no change of state:
    /// the radio refused the first start, and the next one is accepted.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_advertisement_update_is_tried_again() {
        let fixture = fixture("advertising-retry", LARGER_DEVICE_ID);
        let service = fixture.service.clone();
        let driver = Arc::new(FirstAdvertiseFails::new());
        let link = PeerLink::with_driver(
            driver.clone(),
            PeerLinkConfig::new(service.advertiser.advertised_config()),
        );
        let _incoming = link.incoming().unwrap();
        assert!(service.peer_link.set(link).is_ok());
        service.advertiser.set_wanted(true);

        tokio::spawn({
            let service = service.clone();
            async move { service.run_server().await }
        });
        wait_until(
            "the advertisement starts after the refused start",
            Duration::from_secs(5),
            || driver.started.load(Ordering::SeqCst) > 0,
        )
        .await;
        assert!(driver.advertise_calls.load(Ordering::SeqCst) >= 2);
        let _ = service.shutdown().await;
    }

    /// The wait between advertisement retries grows, and has a limit.
    #[test]
    fn advertisement_retries_wait_longer_up_to_a_limit() {
        let waits: Vec<u64> = (1..=8)
            .map(|failed| advertising_retry_wait(failed).as_secs())
            .collect();
        assert_eq!(waits, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    }
}
