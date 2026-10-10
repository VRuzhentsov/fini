//! Who may use the Bluetooth radio, and when: scans, dials, candidate
//! probes and pairing legs take turns on one adapter.
//!
//! One `RadioArbiter` per `DeviceConnectionState`, so two devices in one
//! test process each have their own.
//!
//! It also runs the search coordinator (ADR-0008 D13): one scan for the
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
//! | any wanted peer | presence is recorded (green, D9) |
//! | a peer a delivery or setup search waits for | its address is handed to that search, which dials it |
//!
//! Scanning and dialling never overlap: an adapter cannot usefully drive
//! discovery and establish a connection at once (hardware evidence in
//! `connect_by_advertisement`'s history). A handed-off address carries a
//! `DialGuard`, and the scan does not resume until every guard is dropped.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Once};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, watch, Mutex, MutexGuard, Notify};

use super::{
    advertised_fingerprint, backend, datagram_config, fingerprint_of, note_adapter_reachable,
    note_adapter_unreachable, note_peer_advertising, note_scan_refused, open_db_at_path, FINGERPRINT_LEN,
    FINI_MANUFACTURER_ID, STATUS_SEARCH_WINDOW,
};
use crate::services::communication::pairing::ChannelKind;

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
const FAILED_DIAL_COOLDOWN: Duration = Duration::from_secs(10);

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
    _dial: DialGuard,
}

/// Held while a dial runs. The scan waits for every guard to be dropped.
pub struct DialGuard(Arc<watch::Sender<usize>>);

impl Drop for DialGuard {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// Held while a pairing leg or a Bluetooth channel setup runs; see
/// `RadioArbiter::pairing_legs`.
pub struct PairingLeg(Arc<watch::Sender<usize>>);

impl Drop for PairingLeg {
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

pub struct RadioArbiter {
    /// Pairing legs (request / accept / complete) in progress. The add-mode
    /// candidate scan yields to them: it holds a discovery session and
    /// re-dials the same phone every pass, and an adapter doing either while
    /// a pairing dial runs refuses or stalls it -- ble-gatt answered the pair
    /// dial with "a dial to this peer is already in flight" and one dial hung
    /// until abandoned, so the Pair button reported "Couldn't reach".
    pairing_legs: Arc<watch::Sender<usize>>,
    /// Held for the length of one candidate probe; see `dial_for_pairing`.
    candidate_probe: Mutex<()>,
    /// One Bluetooth scan at a time. Every `backend.scan` call holds this for
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
    scan_lease: Mutex<()>,
    /// Set while a discovery session is actually running: `scan()` returned
    /// Ok and the stream is alive. Holding `scan_lease` is not enough -- the
    /// holder may still be waiting on an adapter that will refuse it.
    scan_running: AtomicBool,
    /// Peers with an exchange attempt in flight -- one at a time per peer.
    exchanges: StdMutex<HashSet<String>>,
    /// Dials running now; the coordinator's scan waits for none.
    dials: Arc<watch::Sender<usize>>,
    requests: StdMutex<Requests>,
    /// Wakes the scan loop whenever what is wanted changes.
    changed: Notify,
    started: Once,
    failed_dials: StdMutex<HashMap<String, Instant>>,
}

impl Default for RadioArbiter {
    fn default() -> Self {
        Self {
            pairing_legs: Arc::new(watch::channel(0).0),
            candidate_probe: Mutex::new(()),
            scan_lease: Mutex::new(()),
            scan_running: AtomicBool::new(false),
            exchanges: StdMutex::new(HashSet::new()),
            dials: Arc::new(watch::channel(0).0),
            requests: StdMutex::new(Requests::default()),
            changed: Notify::new(),
            started: Once::new(),
            failed_dials: StdMutex::new(HashMap::new()),
        }
    }
}

impl RadioArbiter {
    pub fn begin_pairing_leg(&self) -> PairingLeg {
        self.pairing_legs.send_modify(|count| *count += 1);
        PairingLeg(self.pairing_legs.clone())
    }

    /// Follows how many pairing legs are held.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn watch_pairing_legs(&self) -> watch::Receiver<usize> {
        self.pairing_legs.subscribe()
    }

    /// Pairing legs held right now.
    #[cfg(test)]
    pub(crate) fn pairing_legs_held(&self) -> usize {
        *self.pairing_legs.borrow()
    }

    /// Held for the length of one candidate probe. A dial to a peer the
    /// probe may be dialling waits for it, rather than cancelling it.
    pub async fn candidate_probe(&self) -> MutexGuard<'_, ()> {
        self.candidate_probe.lock().await
    }

    pub(super) async fn scan_lease(&self) -> MutexGuard<'_, ()> {
        self.scan_lease.lock().await
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub(super) fn try_scan_lease(&self) -> Option<MutexGuard<'_, ()>> {
        self.scan_lease.try_lock().ok()
    }

    pub(super) fn start_running_scan(&self) -> RunningScan<'_> {
        self.scan_running.store(true, Ordering::SeqCst);
        note_adapter_reachable();
        RunningScan(&self.scan_running)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub(super) fn scan_is_running(&self) -> bool {
        self.scan_running.load(Ordering::SeqCst)
    }

    /// Whether this device is dialing the peer for an exchange right now.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn dialing(&self, peer_id: &str) -> bool {
        self.exchanges.lock().unwrap().contains(peer_id)
    }

    /// Registers an exchange attempt with `peer_id`; false if one runs.
    pub(super) fn begin_exchange(&self, peer_id: &str) -> bool {
        self.exchanges.lock().unwrap().insert(peer_id.to_string())
    }

    pub(super) fn end_exchange(&self, peer_id: &str) {
        self.exchanges.lock().unwrap().remove(peer_id);
    }

    fn dial_guard(&self) -> DialGuard {
        self.dials.send_modify(|count| *count += 1);
        DialGuard(self.dials.clone())
    }

    /// A guard for a dial the coordinator did not hand off: the running
    /// scan sees the guard and stops, and this returns once it has (the
    /// scan lease is free), so the dial never overlaps a scan.
    pub async fn acquire_dial(&self) -> DialGuard {
        let guard = self.dial_guard();
        drop(self.scan_lease().await);
        guard
    }

    /// The scan lease, taken only while no dial is running. For scans
    /// outside the coordinator (the add-mode candidate scan): the lease alone
    /// does not see a dial already under way, and scanning during it breaks
    /// the dial.
    #[cfg(any(feature = "ui-plane", test))]
    pub(super) async fn scan_lease_between_dials(&self) -> MutexGuard<'_, ()> {
        let mut dials = self.dials.subscribe();
        loop {
            let _ = dials.wait_for(|count| *count == 0).await;
            let lease = self.scan_lease().await;
            if *dials.borrow() == 0 {
                return lease;
            }
        }
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

        let found = tokio::time::timeout(window, heard).await.ok().and_then(Result::ok);

        self.requests.lock().unwrap().searches.retain(|request| request.id != id);
        self.changed.notify_one();
        if found.is_some() {
            // The hand-off wakes this task before the coordinator has dropped
            // its discovery stream; the lease comes free only once it has, so
            // the caller's dial never overlaps the scan. `Found` holds a dial
            // guard, so no new scan starts meanwhile.
            drop(self.scan_lease().await);
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

    /// The peers the next scan listens for, by advertised fingerprint.
    fn wanted(&self) -> HashMap<[u8; FINGERPRINT_LEN], String> {
        let (peers, status) = {
            let requests = self.requests.lock().unwrap();
            let peers: Vec<String> = requests.searches.iter().map(|request| request.peer.clone()).collect();
            (peers, requests.status.clone())
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
                        _dial: self.dial_guard(),
                    })
                    .is_ok();
            }
        }
        handed
    }

    /// A dial to `address` after a hand-off failed: the search skips that
    /// address for `FAILED_DIAL_COOLDOWN` and keeps listening for the peer's
    /// current one.
    pub fn note_dial_failed(&self, address: &str) {
        self.failed_dials.lock().unwrap().insert(address.to_string(), Instant::now());
    }

    fn dial_recently_failed(&self, address: &str) -> bool {
        let mut failed = self.failed_dials.lock().unwrap();
        failed.retain(|_, at| at.elapsed() < FAILED_DIAL_COOLDOWN);
        failed.contains_key(address)
    }

    async fn run(self: Arc<Self>) {
        use futures_util::StreamExt;

        let mut dials = self.dials.subscribe();
        loop {
            // Never scan while a dial runs.
            let _ = dials.wait_for(|count| *count == 0).await;

            let mut wanted = self.wanted();
            if wanted.is_empty() {
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

            let Ok(backend) = backend().await else {
                note_adapter_unreachable();
                let _ = tokio::time::timeout(SCAN_FAILURE_PAUSE, self.changed.notified()).await;
                continue;
            };
            let scan = self.scan_lease().await;
            let mut discovered = match backend.scan(datagram_config().service).await {
                Ok(discovered) => discovered,
                Err(err) => {
                    log::warn!("[transport][ble] search scan refused: {err}");
                    note_scan_refused(&err);
                    drop(scan);
                    let _ = tokio::time::timeout(SCAN_FAILURE_PAUSE, self.changed.notified()).await;
                    continue;
                }
            };
            let running = self.start_running_scan();

            // One window at a time: the wanted set is re-read between
            // windows, and at once whenever a search starts or ends.
            let deadline = tokio::time::Instant::now() + STATUS_SEARCH_WINDOW;
            loop {
                tokio::select! {
                    next = tokio::time::timeout_at(deadline, discovered.next()) => match next {
                        Ok(Some(Ok(candidate))) => {
                            let advertised = advertised_fingerprint(
                                candidate
                                    .manufacturer_data
                                    .get(&FINI_MANUFACTURER_ID)
                                    .map(|payload| payload.as_slice()),
                            );
                            let Some(peer) = advertised.and_then(|fp| wanted.get(&fp)) else {
                                continue;
                            };
                            note_peer_advertising(peer, &candidate.address.0);
                            if self.dial_recently_failed(&candidate.address.0) {
                                continue;
                            }
                            if self.hand_off(peer, &candidate.address.0) {
                                break;
                            }
                        }
                        Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                    },
                    // A dial started outside a hand-off: stop for it.
                    _ = dials.wait_for(|count| *count > 0) => break,
                    // What is wanted changed: keep the scan running over the
                    // new set rather than restarting the radio, and stop only
                    // once nothing is wanted any more.
                    _ = self.changed.notified() => {
                        wanted = self.wanted();
                        if wanted.is_empty() {
                            break;
                        }
                    }
                }
            }
            // Stop discovery before any dial the hand-off starts.
            drop(running);
            drop(discovered);
            drop(scan);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searching(radio: &RadioArbiter, peer: &str) -> bool {
        radio.requests.lock().unwrap().searches.iter().any(|request| request.peer == peer)
    }

    /// A dial started outside a hand-off waits until the scan holding the
    /// adapter has stopped, and the scan sees the dial.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dial_waits_for_the_running_scan_to_stop() {
        let radio = Arc::new(RadioArbiter::default());
        let scan = radio.scan_lease().await;
        let dial = tokio::spawn({
            let radio = radio.clone();
            async move { radio.acquire_dial().await }
        });
        while *radio.dials.borrow() == 0 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!dial.is_finished(), "no dial while the scan holds the adapter");

        drop(scan);
        let guard = tokio::time::timeout(Duration::from_secs(2), dial).await.unwrap().unwrap();
        drop(guard);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn searches_for_different_peers_share_one_scan() {
        let radio = Arc::new(RadioArbiter::default());
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
        let radio = Arc::new(RadioArbiter::default());
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
        assert_eq!(*radio.dials.borrow(), 2);

        drop(delivery);
        drop(setup);
        assert!(!searching(&radio, "coord-both"), "a finished search withdraws itself");
    }

    /// A dial that just failed keeps that address out of hand-offs, and only
    /// until the cooldown has passed: the peer's next private address, or the
    /// same one once it answers again, is dialled as usual.
    #[test]
    fn a_failed_address_is_skipped_until_its_cooldown_expires() {
        let radio = RadioArbiter::default();
        radio.note_dial_failed("AA:00:00:00:00:10");
        assert!(radio.dial_recently_failed("AA:00:00:00:00:10"));
        assert!(!radio.dial_recently_failed("AA:00:00:00:00:11"), "only the failed address is skipped");

        let expired = Instant::now()
            .checked_sub(FAILED_DIAL_COOLDOWN)
            .expect("the clock is past one cooldown");
        radio.failed_dials.lock().unwrap().insert("AA:00:00:00:00:10".to_string(), expired);
        assert!(!radio.dial_recently_failed("AA:00:00:00:00:10"), "the cooldown has passed");
        assert!(
            !radio.failed_dials.lock().unwrap().contains_key("AA:00:00:00:00:10"),
            "an expired entry is dropped"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_search_that_hears_nothing_ends_with_its_window_and_withdraws() {
        let radio = Arc::new(RadioArbiter::default());
        let found = radio.find("coord-silent", Purpose::Delivery, Duration::from_millis(50)).await;
        assert!(found.is_none());
        assert!(!searching(&radio, "coord-silent"));
        assert!(!radio.hand_off("coord-silent", "AA:00:00:00:00:04"));
    }
}
