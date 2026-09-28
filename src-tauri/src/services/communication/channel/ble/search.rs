//! The Bluetooth search coordinator (ADR-0008 D13).
//!
//! One scan for the whole app. Callers ask for a search with a purpose --
//! status while the Device page is open, delivery when there is work for a
//! peer, setup while the setup dialog is open -- and the coordinator merges
//! whatever is asked into one running scan over every peer wanted. It never
//! starts the radio twice for the same thing.
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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::Duration;

use tokio::sync::{oneshot, watch, Notify};

use super::{
    advertised_fingerprint, backend, datagram_config, fingerprint_of, note_adapter_unreachable,
    note_peer_advertising, open_db_at_path, scan_lease, RunningScan, FINGERPRINT_LEN,
    FINI_MANUFACTURER_ID, STATUS_SEARCH_WINDOW,
};
use crate::services::communication::pairing::ChannelKind;

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
pub struct DialGuard(());

impl DialGuard {
    pub fn new() -> Self {
        dials().send_modify(|count| *count += 1);
        Self(())
    }

    /// A guard for a dial the coordinator did not hand off: the running
    /// scan sees the guard and stops, and this returns once it has (the
    /// scan lease is free), so the dial never overlaps a scan.
    pub async fn acquire() -> Self {
        let guard = Self::new();
        drop(super::scan_lease().lock().await);
        guard
    }
}

impl Drop for DialGuard {
    fn drop(&mut self) {
        dials().send_modify(|count| *count = count.saturating_sub(1));
    }
}

fn dials() -> &'static watch::Sender<usize> {
    static DIALS: OnceLock<watch::Sender<usize>> = OnceLock::new();
    DIALS.get_or_init(|| watch::channel(0).0)
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

fn requests() -> &'static StdMutex<Requests> {
    static REQUESTS: OnceLock<StdMutex<Requests>> = OnceLock::new();
    REQUESTS.get_or_init(|| StdMutex::new(Requests::default()))
}

/// Wakes the scan loop whenever what is wanted changes.
fn changed() -> &'static Notify {
    static CHANGED: OnceLock<Notify> = OnceLock::new();
    CHANGED.get_or_init(Notify::new)
}

fn start_once() {
    // Unit tests drive `hand_off` directly; a real scan loop there would
    // reach for an adapter the test machine does not have.
    if cfg!(test) {
        return;
    }
    static STARTED: std::sync::Once = std::sync::Once::new();
    // Tauri's runtime, not `tokio::spawn`: this is reached from synchronous
    // commands, which run on the main thread outside any Tokio context.
    STARTED.call_once(|| {
        tauri::async_runtime::spawn(run());
    });
}

/// Search for `peer` for up to `window`, ending early once it is heard.
pub async fn find(peer: &str, purpose: Purpose, window: Duration) -> Option<Found> {
    let (reply, heard) = oneshot::channel();
    let id = {
        let mut requests = requests().lock().unwrap();
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
    start_once();
    changed().notify_one();

    let found = tokio::time::timeout(window, heard).await.ok().and_then(Result::ok);

    requests().lock().unwrap().searches.retain(|request| request.id != id);
    changed().notify_one();
    if found.is_some() {
        // The hand-off wakes this task before the coordinator has dropped
        // its discovery stream; the lease comes free only once it has, so
        // the caller's dial never overlaps the scan. `Found` holds a dial
        // guard, so no new scan starts meanwhile.
        drop(super::scan_lease().lock().await);
    }
    found
}

/// Turn the status search on while the Device page is open, off when it
/// closes (ADR-0008 D12).
#[cfg(any(feature = "ui-plane", test))]
pub fn set_status(db_path: Option<PathBuf>) {
    requests().lock().unwrap().status = db_path;
    start_once();
    changed().notify_one();
}

/// The peers the next scan listens for, by advertised fingerprint.
fn wanted() -> HashMap<[u8; FINGERPRINT_LEN], String> {
    let (peers, status) = {
        let requests = requests().lock().unwrap();
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
fn hand_off(peer: &str, address: &str) -> bool {
    let mut requests = requests().lock().unwrap();
    let mut handed = false;
    for request in requests.searches.iter_mut().filter(|request| request.peer == peer) {
        if let Some(reply) = request.reply.take() {
            log::info!("[transport][ble] {:?} search heard {peer} at {address}", request.purpose);
            handed |= reply
                .send(Found {
                    address: address.to_string(),
                    _dial: DialGuard::new(),
                })
                .is_ok();
        }
    }
    handed
}

/// Pause after the radio refused a scan, so a broken adapter is not hammered.
const SCAN_FAILURE_PAUSE: Duration = Duration::from_secs(5);

async fn run() {
    use futures_util::StreamExt;

    let mut dials = dials().subscribe();
    loop {
        // Never scan while a dial runs.
        let _ = dials.wait_for(|count| *count == 0).await;

        let mut wanted = wanted();
        if wanted.is_empty() {
            // With the page open, a channel set up meanwhile joins the next
            // window; otherwise only a new search can change anything.
            if requests().lock().unwrap().status.is_some() {
                let _ = tokio::time::timeout(STATUS_SEARCH_WINDOW, changed().notified()).await;
            } else {
                changed().notified().await;
            }
            continue;
        }

        let Ok(backend) = backend().await else {
            note_adapter_unreachable();
            let _ = tokio::time::timeout(SCAN_FAILURE_PAUSE, changed().notified()).await;
            continue;
        };
        let scan = scan_lease().lock().await;
        let Ok(mut discovered) = backend.scan(datagram_config().service).await else {
            note_adapter_unreachable();
            drop(scan);
            let _ = tokio::time::timeout(SCAN_FAILURE_PAUSE, changed().notified()).await;
            continue;
        };
        let running = RunningScan::start();

        // One window at a time: the wanted set is re-read between windows,
        // and at once whenever a search starts or ends.
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
                        if hand_off(peer, &candidate.address.0) {
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
                _ = changed().notified() => {
                    wanted = self::wanted();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn searching(peer: &str) -> bool {
        requests().lock().unwrap().searches.iter().any(|request| request.peer == peer)
    }

    /// A dial started outside a hand-off waits until the scan holding the
    /// adapter has stopped, and the scan sees the dial.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dial_waits_for_the_running_scan_to_stop() {
        let scan = super::super::scan_lease().lock().await;
        let dial = tokio::spawn(DialGuard::acquire());
        while *dials().borrow() == 0 {
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
        let first = tokio::spawn(find("coord-merge-a", Purpose::Delivery, Duration::from_secs(5)));
        let second = tokio::spawn(find("coord-merge-b", Purpose::Setup, Duration::from_secs(5)));
        while !(searching("coord-merge-a") && searching("coord-merge-b")) {
            tokio::task::yield_now().await;
        }

        let wanted = wanted();
        assert_eq!(wanted.get(&fingerprint_of("coord-merge-a")).map(String::as_str), Some("coord-merge-a"));
        assert_eq!(wanted.get(&fingerprint_of("coord-merge-b")).map(String::as_str), Some("coord-merge-b"));

        assert!(hand_off("coord-merge-a", "AA:00:00:00:00:01"));
        assert!(hand_off("coord-merge-b", "AA:00:00:00:00:02"));
        assert_eq!(first.await.unwrap().map(|found| found.address).as_deref(), Some("AA:00:00:00:00:01"));
        assert_eq!(second.await.unwrap().map(|found| found.address).as_deref(), Some("AA:00:00:00:00:02"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_heard_peer_reaches_every_search_waiting_for_it_and_pauses_the_scan() {
        let delivery = tokio::spawn(find("coord-both", Purpose::Delivery, Duration::from_secs(5)));
        let setup = tokio::spawn(find("coord-both", Purpose::Setup, Duration::from_secs(5)));
        while requests().lock().unwrap().searches.iter().filter(|r| r.peer == "coord-both").count() < 2 {
            tokio::task::yield_now().await;
        }

        let dials_before = *dials().borrow();
        assert!(hand_off("coord-both", "AA:00:00:00:00:03"));
        let delivery = delivery.await.unwrap().expect("delivery search hears the peer");
        let setup = setup.await.unwrap().expect("setup search hears the peer");
        assert_eq!(delivery.address, "AA:00:00:00:00:03");
        assert_eq!(setup.address, "AA:00:00:00:00:03");
        // Scanning waits while either dial is possible.
        assert!(*dials().borrow() >= dials_before + 2);

        drop(delivery);
        drop(setup);
        assert!(!searching("coord-both"), "a finished search withdraws itself");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_search_that_hears_nothing_ends_with_its_window_and_withdraws() {
        let found = find("coord-silent", Purpose::Delivery, Duration::from_millis(50)).await;
        assert!(found.is_none());
        assert!(!searching("coord-silent"));
        assert!(!hand_off("coord-silent", "AA:00:00:00:00:04"));
    }
}
