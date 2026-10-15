//! SpaceSync: carries the changes of mapped spaces to paired devices.
//!
//! One object per `DeviceConnectionState`. It owns when sync runs (the
//! wake signal, the keeper, the time of the last tick), how often one
//! event is sent again, the sessions on the links the channels hold
//! (`hold_session`), and which green channel a frame goes over
//! (`channel_order_for`). What it does on each run is
//! `commands::space_sync_tick_impl`.
//!
//! Work for a device without a green channel waits: the channels connect
//! by themselves (`ChannelService::keep_serving`), and a session coming up
//! wakes sync (`DeviceConnectionState::try_claim_session`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use diesel::sqlite::SqliteConnection;

use crate::services::communication::channel::DataLink;
use crate::services::communication::pairing::{channels, ChannelKind, DeviceConnectionState};

/// The longest an idle, fully-connected pair may go without a tick on
/// Android.
///
/// A floor for the *unprompted* cadence, not the latency of a change: every
/// moment that makes work possible calls [`SpaceSync::wake`] instead --
/// a local edit, an inbound frame, a session being claimed, a channel
/// switched on, a peer becoming reachable. Each of those fires on a
/// *transition*, never on the steady state that follows, or the backstop
/// becomes a poll wearing a different name. It was 3s (issue #171:
/// ~28,800 wakeups a day for a handful of quest changes).
#[cfg(target_os = "android")]
const TICK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How long the keeper waits after a wake before it ticks: frames arrive
/// together, and each one wakes, so a burst becomes one tick.
const KEEPER_SETTLE: Duration = Duration::from_millis(150);

/// Minimum interval between two sends of the *same* unacked event to the
/// same device. Without it an event that is slow to be acked is sent again
/// on every tick; on Bluetooth that overran the peripheral's inbound queue,
/// the link dropped, and with it the ack that would have stopped the
/// resends (actors-ble e2e lane).
const EVENT_RESEND_COOLDOWN: Duration = Duration::from_secs(5);

pub struct SpaceSync {
    /// Raised when there is sync work, in either direction. A `Notify`
    /// because the payload is irrelevant: the tick reads the outbox from
    /// SQLite anyway. `notify_one`, not `notify_waiters`: a wake raised
    /// while the keeper is mid-tick must survive until it next waits; at
    /// worst the keeper runs one extra pass that finds nothing.
    wake: Arc<tokio::sync::Notify>,
    /// When the last tick ran, from any caller. `None` until the first.
    last_tick_at: Mutex<Option<Instant>>,
    /// When each event was last sent to each device, keyed by
    /// `(peer_device_id, event_id)`: one event can be pending for several
    /// devices, and each delivery is independent. In memory only: resend
    /// pacing matters only within one process.
    sent_at: Mutex<HashMap<(String, String), Instant>>,
    /// Per paired device, the channels in the order a frame goes over them:
    /// the primary first (ADR-0007). Read from the database on each tick
    /// (`note_channel_order`); Network first until then.
    channel_order: Mutex<HashMap<String, [ChannelKind; 2]>>,
    keeper_started: Once,
    /// Set by the first tick the frontend asks for (`commands::
    /// space_sync_tick`). On Android the Activity context that the
    /// Bluetooth adapter and the foreground service need exists only from
    /// then on; ticks before it serve Network alone.
    frontend_ticked: std::sync::atomic::AtomicBool,
}

impl Default for SpaceSync {
    fn default() -> Self {
        Self {
            wake: Arc::new(tokio::sync::Notify::new()),
            last_tick_at: Mutex::new(None),
            sent_at: Mutex::new(HashMap::new()),
            channel_order: Mutex::new(HashMap::new()),
            keeper_started: Once::new(),
            frontend_ticked: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl SpaceSync {
    /// There is sync work: run a tick soon. Infallible and sync, so it can
    /// be called inside a write path; a missed wake costs latency, never
    /// data.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Wake once `delay` has passed: work that is due later (a retry
    /// schedule, a cooldown), which only a tick acts on.
    pub fn wake_after(&self, delay: Duration) {
        let wake = self.wake.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            wake.notify_one();
        });
    }

    /// The frontend asked for a tick: on Android the Activity is up.
    #[cfg(any(feature = "ui-plane", test))]
    pub(super) fn note_frontend_tick(&self) {
        self.frontend_ticked.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether a tick may use what needs the Android Activity (the Bluetooth
    /// channel, the foreground service). Always true off Android.
    pub(super) fn activity_ready(&self) -> bool {
        cfg!(not(target_os = "android")) || self.frontend_ticked.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn note_tick_ran(&self) {
        if let Ok(mut at) = self.last_tick_at.lock() {
            *at = Some(Instant::now());
        }
    }

    #[cfg(target_os = "android")]
    fn tick_is_overdue(&self) -> bool {
        match self.last_tick_at.lock() {
            Ok(at) => at.map(|at| at.elapsed() >= TICK_INTERVAL).unwrap_or(true),
            // Poisoned only if a tick panicked; ticking again is safer.
            Err(_) => true,
        }
    }

    /// Whether `event_id` may be sent to `peer_device_id` now: it was not
    /// sent to that device within `EVENT_RESEND_COOLDOWN`.
    pub(super) fn may_send(&self, peer_device_id: &str, event_id: &str, now: Instant) -> bool {
        let sent_at = self.sent_at.lock().unwrap();
        sent_at
            .get(&(peer_device_id.to_string(), event_id.to_string()))
            .is_none_or(|&last| now.duration_since(last) >= EVENT_RESEND_COOLDOWN)
    }

    /// The event was handed to the device's exchange. Only a send that
    /// succeeded starts the cooldown.
    pub(super) fn note_sent(&self, peer_device_id: &str, event_id: &str, now: Instant) {
        self.sent_at
            .lock()
            .unwrap()
            .insert((peer_device_id.to_string(), event_id.to_string()), now);
    }

    /// Starts the keeper: the task that ticks when woken, and on Android
    /// also when no tick ran within `TICK_INTERVAL` (a backgrounded WebView
    /// has its timers throttled). On desktop it only wakes on a signal: a
    /// periodic second ticker there only contended with the app for the
    /// SQLite write lock (21 keeper ticks, 21 `database is locked`).
    ///
    /// Started when the app starts (`lib.rs` setup), so held connections
    /// come up without waiting for the frontend to load -- that took about
    /// 25 s on a phone with the dev frontend. What needs the Android
    /// Activity waits for the frontend's first tick (`activity_ready`).
    pub(crate) fn start_keeper_once(&self, state: &DeviceConnectionState) {
        self.keeper_started.call_once(|| {
            let state = state.clone();
            tauri::async_runtime::spawn(async move {
                let space_sync = state.space_sync.clone();
                loop {
                    // Registered *before* the wait, so a wake raised while
                    // this task was mid-tick is not lost.
                    let woken = space_sync.wake.notified();

                    #[cfg(target_os = "android")]
                    {
                        tokio::select! {
                            _ = tokio::time::sleep(TICK_INTERVAL) => {
                                if !space_sync.tick_is_overdue() {
                                    continue;
                                }
                            }
                            _ = woken => {}
                        }
                    }
                    #[cfg(not(target_os = "android"))]
                    {
                        woken.await;
                    }
                    tokio::time::sleep(KEEPER_SETTLE).await;

                    let state = state.clone();
                    // The tick is blocking (SQLite plus the exchanges it
                    // starts), so it runs off the async workers, on its own
                    // connection.
                    let result = tokio::task::spawn_blocking(move || {
                        let mut conn = crate::services::db::open_db_at_path(&state.db_path);
                        super::commands::space_sync_tick_impl(&mut conn, &state)
                    })
                    .await;
                    match result {
                        Ok(Err(err)) => log::warn!("[space_sync][keeper] tick failed: {err}"),
                        Err(err) => log::warn!("[space_sync][keeper] tick task failed: {err}"),
                        Ok(Ok(_)) => {}
                    }
                }
            });
        });
    }

    /// Runs the sync session on a held link to `peer_device_id` over `kind`.
    /// The channels and the gate hand every live, TLS-proven link here; they
    /// do not know what a session sends. At most one session per (device,
    /// channel): `Err(link)` gives the link back when one already runs, so
    /// the caller can refuse it. `on_held` runs once the session is claimed,
    /// before it starts. Returns the close code the peer ended the link
    /// with, if it did (`session::refused_by_peer`).
    pub(crate) async fn hold_session(
        &self,
        state: &DeviceConnectionState,
        link: Box<dyn DataLink>,
        peer_device_id: &str,
        kind: ChannelKind,
        on_held: impl FnOnce(),
    ) -> Result<Option<u32>, Box<dyn DataLink>> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        if !state.try_claim_session(peer_device_id, kind, tx) {
            return Err(link);
        }
        on_held();
        Ok(super::session::run_session(
            link,
            rx,
            state.clone(),
            state.db_path.clone(),
            peer_device_id.to_string(),
            super::session::idle_after(kind),
        )
        .await)
    }

    /// Reads, for each paired device, which channel a frame goes over first
    /// (the primary, ADR-0007). Called on each tick, so a changed primary
    /// takes effect on the next one.
    pub(super) fn note_channel_order(&self, conn: &mut SqliteConnection, peer_device_ids: &[String]) {
        let orders: Vec<(String, [ChannelKind; 2])> = peer_device_ids
            .iter()
            .map(|peer| (peer.clone(), Self::channel_order(channels::primary_kind(conn, peer))))
            .collect();
        if let Ok(mut order) = self.channel_order.lock() {
            order.clear();
            order.extend(orders);
        }
    }

    /// The channels a frame for `peer_device_id` goes over, in order:
    /// `DeviceConnectionState::push_to_peer` uses the first one with a
    /// session.
    pub(crate) fn channel_order_for(&self, peer_device_id: &str) -> [ChannelKind; 2] {
        self.channel_order
            .lock()
            .ok()
            .and_then(|order| order.get(peer_device_id).copied())
            .unwrap_or_else(|| Self::channel_order(None))
    }

    /// Which channels to try, in order: the primary first (ADR-0007),
    /// otherwise Network first.
    pub(super) fn channel_order(primary: Option<ChannelKind>) -> [ChannelKind; 2] {
        match primary {
            Some(ChannelKind::Bluetooth) => [ChannelKind::Bluetooth, ChannelKind::Network],
            _ => [ChannelKind::Network, ChannelKind::Bluetooth],
        }
    }
}
