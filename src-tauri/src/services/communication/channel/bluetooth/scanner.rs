//! The scanner: the one Bluetooth scan of the app, and what it heard.
//!
//! It gives the radio to one task at a time: a scan, a connection, an
//! identify step, or a pairing step. It also keeps what the scan heard:
//! the devices available for Add Device (`AvailableDevices`), and when and
//! where each paired device was last heard.
//!
//! One `Scanner` per `DeviceConnectionState`, so two devices in one
//! test process each have their own.
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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Once};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, watch, Mutex, MutexGuard, Notify};

#[cfg(any(feature = "ui-plane", test))]
use super::{connect, recv_frame, send_frame, PeerFrame, ADD_MODE_FLAG_BYTE};
use super::{
    advertised_fingerprint, datagram_config, fingerprint_of, open_db_at_path, BluetoothAdapter,
    FINGERPRINT_LEN, FINI_MANUFACTURER_ID, STATUS_SEARCH_WINDOW,
};

/// How long a heard advertisement keeps a paired device present: three
/// status-search windows, so one missed window does not flicker the row
/// (ADR-0008 D12).
const BLUETOOTH_CHANNEL_TIMEOUT: Duration = Duration::from_secs(60);
use crate::services::communication::pairing::ChannelKind;
use crate::services::communication::sync::space_sync::SpaceSync;

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

/// Pause after the radio refused a scan, so a broken adapter is not hammered.
const SCAN_FAILURE_PAUSE: Duration = Duration::from_secs(5);

/// Why a search runs (ADR-0008 D12). The coordinator treats delivery and
/// setup alike -- hand the peer's address over -- and keeps the purpose for
/// the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Delivery,
    #[cfg(any(feature = "ui-plane", test))]
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
    /// Set while the Device page is open: every paired peer with Bluetooth
    /// on is wanted, for presence. The database the peers are read from.
    status: Option<PathBuf>,
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
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) available: AvailableDevices,
    /// Woken when a paired device is heard again.
    space_sync: Arc<SpaceSync>,
}

#[cfg(test)]
impl Default for Scanner {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl Scanner {
    pub fn new(space_sync: Arc<SpaceSync>) -> Self {
        Self {
            space_sync,
            pairing_steps: Arc::new(watch::channel(0).0),
            candidate_probe: Mutex::new(()),
            scan_lock: Mutex::new(()),
            scan_running: AtomicBool::new(false),
            exchanges: StdMutex::new(HashSet::new()),
            connections: Arc::new(watch::channel(0).0),
            requests: StdMutex::new(Requests::default()),
            changed: Notify::new(),
            started: Once::new(),
            failed_connects: StdMutex::new(HashMap::new()),
            paired_heard: StdMutex::new(HashMap::new()),
            add_device_mode: AtomicBool::new(false),
            #[cfg(any(feature = "ui-plane", test))]
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
        // must not make the keeper poll.
        if newly_present {
            self.space_sync.wake();
        }
    }

    /// Where `peer_id` was last heard advertising, if within the channel
    /// timeout: a present device is reached by one connection, not a search.
    pub fn last_heard_address(&self, peer_id: &str) -> Option<String> {
        let heard = self.paired_heard.lock().ok()?;
        heard
            .get(peer_id)
            .filter(|(at, _)| at.elapsed() < BLUETOOTH_CHANNEL_TIMEOUT)
            .map(|(_, address)| address.clone())
    }

    /// Whether `peer_id` advertised within the channel timeout (ADR-0008 D9).
    pub fn heard_recently(&self, peer_id: &str) -> bool {
        self.last_heard_address(peer_id).is_some()
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

    #[cfg(any(feature = "ui-plane", test))]
    pub(super) fn try_scan_lock(&self) -> Option<MutexGuard<'_, ()>> {
        self.scan_lock.try_lock().ok()
    }

    pub(super) fn start_running_scan(&self) -> RunningScan<'_> {
        self.scan_running.store(true, Ordering::SeqCst);
        BluetoothAdapter::this_device().note_reachable();
        RunningScan(&self.scan_running)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub(super) fn scan_is_running(&self) -> bool {
        self.scan_running.load(Ordering::SeqCst)
    }

    /// Whether this device is connecting to the peer for an exchange right
    /// now.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn connecting(&self, peer_id: &str) -> bool {
        self.exchanges.lock().unwrap().contains(peer_id)
    }

    /// Registers an exchange attempt with `peer_id`; false if one runs.
    pub(super) fn begin_exchange(&self, peer_id: &str) -> bool {
        self.exchanges.lock().unwrap().insert(peer_id.to_string())
    }

    pub(super) fn end_exchange(&self, peer_id: &str) {
        self.exchanges.lock().unwrap().remove(peer_id);
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

    /// Turn the status search on while the Device page is open, off when it
    /// closes (ADR-0008 D12).
    #[cfg(any(feature = "ui-plane", test))]
    pub fn set_status(self: &Arc<Self>, db_path: Option<PathBuf>) {
        self.requests.lock().unwrap().status = db_path;
        self.start_once();
        self.changed.notify_one();
    }

    /// Add Device opened or closed. While it is open, the scan runs without
    /// a stop, records every Fini device it hears (`AvailableDevices`), and
    /// serves only setup searches: status and delivery searches wait, so
    /// that the radio stays on the scan the person is watching.
    #[cfg(any(feature = "ui-plane", test))]
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
        let (peers, status) = {
            let requests = self.requests.lock().unwrap();
            let peers: Vec<String> = requests
                .searches
                .iter()
                .filter(|request| !adding || request.purpose != Purpose::Delivery)
                .map(|request| request.peer.clone())
                .collect();
            (peers, requests.status.clone().filter(|_| !adding))
        };
        let mut wanted: HashMap<[u8; FINGERPRINT_LEN], String> =
            peers.into_iter().map(|peer| (fingerprint_of(&peer), peer)).collect();
        if let Some(db_path) = status {
            let on = tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db_path);
                crate::services::communication::pairing::channels::peers_with_channel_enabled(
                    &mut conn,
                    ChannelKind::Bluetooth,
                )
            });
            wanted.extend(on.into_iter().map(|peer| (fingerprint_of(&peer), peer)));
        }
        wanted
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
                // With the page open, a channel set up meanwhile joins the
                // next window; otherwise only a new search can change
                // anything.
                if self.requests.lock().unwrap().status.is_some() {
                    let _ = tokio::time::timeout(STATUS_SEARCH_WINDOW, self.changed.notified()).await;
                } else {
                    self.changed.notified().await;
                }
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
                "[transport][ble] shared scan window started for {} device(s), status search {}, add device {}",
                wanted.len(),
                if self.requests.lock().unwrap().status.is_some() { "on" } else { "off" },
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
                STATUS_SEARCH_WINDOW
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
                            #[cfg(any(feature = "ui-plane", test))]
                            if self.in_add_device_mode() {
                                if let Some(fingerprint) = advertised {
                                    let add_device_flag = payload
                                        .and_then(|payload| payload.first())
                                        .is_some_and(|flags| flags & ADD_MODE_FLAG_BYTE != 0);
                                    self.available.record_heard(
                                        fingerprint,
                                        &candidate.address.0,
                                        candidate.name.clone(),
                                        add_device_flag,
                                    );
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

// ---- Identify step ----

/// How long the identify step waits for the answer once the link is open.
/// The connection itself is never cut short: ble-gatt gives it its own 20 s,
/// and an abandoned connect makes BlueZ crash (two crashes in the spike).
#[cfg(any(feature = "ui-plane", test))]
const IDENTIFY_ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(any(feature = "ui-plane", test))]
impl Scanner {
    /// Pair pressed on a device the Add Device list shows from its
    /// advertisement: one connection to the newest address the scan heard it
    /// at, to learn its device id, name and iroh key. `hello` is this
    /// device's identify message, `own_device_id` its device id. The result
    /// is recorded, so the pair request that follows can connect over iroh.
    pub async fn identify_heard_device(
        &self, fingerprint: [u8; FINGERPRINT_LEN], hello: &PeerFrame, own_device_id: &str,
    ) -> Result<AvailableDevice, String> {
        let address = self
            .available
            .heard_address(fingerprint)
            .ok_or_else(|| "that device is not heard any more -- open Add Device on it again".to_string())?;
        let started = Instant::now();
        let reply = self.identify_device(hello, &address).await;
        log::info!(
            "[transport][ble] identify {address} took {:?}: {}",
            started.elapsed(),
            if reply.is_some() { "answered" } else { "no answer" }
        );
        match reply {
            Some(PeerFrame::DeviceIdentified { device_id, hostname, endpoint_id }) => {
                if device_id == own_device_id {
                    return Err("that is this device".to_string());
                }
                let device = AvailableDevice { address, device_id, hostname };
                self.available.record_identified(&device, endpoint_id);
                Ok(device)
            }
            _ => Err("the device did not answer -- make sure Add Device is open on it".to_string()),
        }
    }

    /// Connects to `address` and exchanges `IdentifyDevice`/`DeviceIdentified`.
    /// `None` on any failure along the way (dial, send, no/wrong reply).
    async fn identify_device(&self, hello: &PeerFrame, address: &str) -> Option<PeerFrame> {
        // A search for a paired device may start a scan; holding a connect
        // guard keeps it paused until this step ends.
        let _probe = self.candidate_probe().await;
        let _connect = self.acquire_connect().await;
        let mut link = connect(address).await.ok()?;
        send_frame(link.as_mut(), hello).await.ok()?;
        tokio::time::timeout(IDENTIFY_ANSWER_TIMEOUT, recv_frame(link.as_mut())).await.ok()??.ok()
    }
}

/// A Fini device near this one that is in Add Device mode, as an identify
/// step learned it.
#[cfg(any(feature = "ui-plane", test))]
#[derive(Clone)]
pub struct AvailableDevice {
    pub address: String,
    pub device_id: String,
    pub hostname: String,
}

// ---- Devices available for Add Device ----

/// The Fini devices around this one while Add Device is open: what their
/// advertisements told the scan, and what an identify step learned.
#[cfg(any(feature = "ui-plane", test))]
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
#[cfg(any(feature = "ui-plane", test))]
const HEARD_FRESH: Duration = Duration::from_secs(15);

/// What one advertisement told the scan.
#[cfg(any(feature = "ui-plane", test))]
#[derive(Clone)]
struct HeardDevice {
    address: String,
    name: Option<String>,
    add_device_flag: bool,
    at: Instant,
}

/// A device for the Add Device list, from the advertisement alone.
#[cfg(any(feature = "ui-plane", test))]
#[derive(Clone, Debug)]
pub struct HeardInAddDevice {
    pub fingerprint: [u8; FINGERPRINT_LEN],
    pub address: String,
    /// The short name from the scan response, if the device sent one.
    pub name: Option<String>,
    /// The device id and full name, if an identify step learned them.
    pub identified: Option<(String, String)>,
}

#[cfg(any(feature = "ui-plane", test))]
impl AvailableDevices {
    /// Forgets what the scan heard, so that a new Add Device starts empty:
    /// a device listed now must be in Add Device now.
    pub fn remove_heard(&self) {
        if let Ok(mut heard) = self.heard.lock() {
            heard.clear();
        }
    }

    /// Records one advertisement: the newest address of that device wins.
    pub fn record_heard(
        &self, fingerprint: [u8; FINGERPRINT_LEN], address: &str, name: Option<String>, add_device_flag: bool,
    ) {
        let Ok(mut heard) = self.heard.lock() else { return };
        let name = name.or_else(|| heard.get(&fingerprint).and_then(|known| known.name.clone()));
        heard.insert(
            fingerprint,
            HeardDevice { address: address.to_string(), name, add_device_flag, at: Instant::now() },
        );
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
