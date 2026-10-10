//! Which paired peers this device has heard over Bluetooth, and when it
//! searches for one again.
//!
//! One `Presence` per `DeviceConnectionState`, shared with its
//! `RadioArbiter`, whose scan is what hears the peers.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

/// How long a heard advertisement keeps a peer present: three status-search
/// windows, so one missed window does not flicker the row (ADR-0008 D12).
const BLUETOOTH_CHANNEL_TIMEOUT: Duration = Duration::from_secs(60);

/// When a delivery search does not find the peer, the next one waits this
/// long, one step further each time and then the last step forever: 1, 5
/// and 15 minutes, then every 15 (ADR-0008 D12). A first thoughtful guess
/// for battery, to be revised from measurements on hardware.
const DELIVERY_RETRY: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];

/// When the next delivery search is due after `misses` searches in a row
/// found nothing.
fn next_delivery_search(misses: usize, now: Instant) -> Instant {
    now + DELIVERY_RETRY[misses.saturating_sub(1).min(DELIVERY_RETRY.len() - 1)]
}

#[derive(Default)]
pub struct Presence {
    /// When each peer was last heard advertising a matching fingerprint, and
    /// at which address. Only ever written from a fingerprint match, so
    /// "heard" means "heard advertising *as this peer*".
    last_seen: StdMutex<HashMap<String, (Instant, String)>>,
    /// Per peer: how many delivery searches in a row found nothing, and when
    /// the next one is due.
    delivery_schedule: StdMutex<HashMap<String, (usize, Instant)>>,
}

impl Presence {
    pub fn note_advertising(&self, peer_id: &str, address: &str) {
        let newly_present = match self.last_seen.lock() {
            Ok(mut seen) => seen
                .insert(peer_id.to_string(), (Instant::now(), address.to_string()))
                .is_none_or(|(previous, _)| previous.elapsed() >= BLUETOOTH_CHANNEL_TIMEOUT),
            Err(_) => false,
        };
        // A peer appearing is the moment waiting work becomes deliverable
        // (ADR-0008 D8). Only on the transition: a peer heard on every
        // window must not turn the keeper into a poll.
        if newly_present {
            crate::services::communication::sync::commands::notify_sync_work_pending();
        }
    }

    /// Where `peer_id` was last heard advertising, if within the channel
    /// timeout: a present peer is reached by one dial, not a search.
    pub fn last_seen_address(&self, peer_id: &str) -> Option<String> {
        match self.last_seen.lock() {
            Ok(seen) => seen
                .get(peer_id)
                .filter(|(at, _)| at.elapsed() < BLUETOOTH_CHANNEL_TIMEOUT)
                .map(|(_, address)| address.clone()),
            Err(_) => None,
        }
    }

    /// Whether `peer_id` advertised within the channel timeout (ADR-0008 D9).
    pub fn seen_recently(&self, peer_id: &str) -> bool {
        match self.last_seen.lock() {
            Ok(seen) => seen
                .get(peer_id)
                .is_some_and(|(at, _)| at.elapsed() < BLUETOOTH_CHANNEL_TIMEOUT),
            Err(_) => false,
        }
    }

    /// Whether a delivery search for this peer is due. A present peer is
    /// always due: connecting to it is one dial, not a search.
    pub fn delivery_due(&self, peer_id: &str) -> bool {
        if self.seen_recently(peer_id) {
            return true;
        }
        match self.delivery_schedule.lock() {
            Ok(schedule) => schedule.get(peer_id).is_none_or(|(_, next)| Instant::now() >= *next),
            Err(_) => true,
        }
    }

    pub fn note_delivery_missed(&self, peer_id: &str) {
        let now = Instant::now();
        let next = match self.delivery_schedule.lock() {
            Ok(mut schedule) => {
                let misses = schedule.get(peer_id).map_or(0, |(misses, _)| *misses) + 1;
                let next = next_delivery_search(misses, now);
                schedule.insert(peer_id.to_string(), (misses, next));
                next
            }
            Err(_) => return,
        };
        // The next search is due then; wake the keeper for it (ADR-0008 D12).
        crate::services::communication::sync::commands::notify_sync_work_pending_after(
            next.saturating_duration_since(now),
        );
    }

    /// The peer was reached, or see `ChannelService::forget_failures`: the
    /// next delivery search is due at once. Also used when a pair is removed.
    pub fn forget_delivery_misses(&self, peer_id: &str) {
        if let Ok(mut schedule) = self.delivery_schedule.lock() {
            schedule.remove(peer_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0008 D12: a peer not found is searched for again after 1, 5 and
    /// 15 minutes, then every 15 minutes.
    #[test]
    fn delivery_searches_back_off_one_five_fifteen_then_every_fifteen() {
        let now = Instant::now();
        let waits: Vec<u64> = (1..=5)
            .map(|misses| next_delivery_search(misses, now).duration_since(now).as_secs())
            .collect();
        assert_eq!(waits, vec![60, 300, 900, 900, 900]);
    }

    /// Switching Bluetooth on tries again at once, whatever retry delay
    /// earlier misses left behind.
    #[test]
    fn forgetting_delivery_misses_makes_the_next_search_due_now() {
        let presence = Presence::default();
        let peer = "peer-forget-delivery-misses";
        presence.note_delivery_missed(peer);
        assert!(!presence.delivery_due(peer), "a miss delays the next search");
        presence.forget_delivery_misses(peer);
        assert!(presence.delivery_due(peer));
    }
}
