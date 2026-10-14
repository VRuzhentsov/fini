//! SpaceSync: carries the changes of mapped spaces to paired devices.
//!
//! One object per `DeviceConnectionState`. It owns when sync runs (the
//! wake signal, the keeper, the time of the last tick), how often one
//! event is sent again, and which channel an exchange goes over. What it
//! does on each run is `commands::space_sync_tick_impl`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use diesel::sqlite::SqliteConnection;

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
    keeper_started: Once,
}

impl Default for SpaceSync {
    fn default() -> Self {
        Self {
            wake: Arc::new(tokio::sync::Notify::new()),
            last_tick_at: Mutex::new(None),
            sent_at: Mutex::new(HashMap::new()),
            keeper_started: Once::new(),
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
    /// Started from the first tick, not from `.setup()`: on Android that
    /// first tick is the earliest point the Bluetooth peripheral can start
    /// (`BluetoothChannel::start_peripheral_once`).
    pub(super) fn start_keeper_once(&self, state: &DeviceConnectionState) {
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

    /// Starts an exchange with each device that has work and none running
    /// (ADR-0008 D10). The primary channel (ADR-0007) is tried first, then
    /// the other; without a primary, Network first. A channel is taken when
    /// the device is present on it and it is not failing; if neither is,
    /// Bluetooth's delivery search looks for the device (D12). A channel
    /// that is not `On` for the pair is never used.
    pub(super) fn request_exchanges(
        &self,
        conn: &mut SqliteConnection,
        state: &DeviceConnectionState,
        peers_with_work: &std::collections::HashSet<String>,
    ) {
        for peer_device_id in peers_with_work {
            if state.has_session(peer_device_id) {
                continue;
            }
            let order = Self::channel_order(channels::primary_kind(conn, peer_device_id));
            let reachable_now = order.into_iter().find(|&kind| {
                let channel = state.channel(kind);
                channels::is_enabled(conn, peer_device_id, kind)
                    && channel.is_present(state, peer_device_id)
                    && !channel.recently_failed(peer_device_id)
            });
            if let Some(kind) = reachable_now {
                state.channel(kind).request_exchange(state, peer_device_id);
            } else if channels::is_enabled(conn, peer_device_id, ChannelKind::Bluetooth) {
                state.channel(ChannelKind::Bluetooth).request_exchange(state, peer_device_id);
            }
        }
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
