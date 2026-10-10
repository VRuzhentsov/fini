//! What this device knows about the Fini devices around it in add-mode.
//!
//! One `PeerDirectory` per `DeviceConnectionState`, so two devices in one
//! test process each keep their own.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{fingerprint_of, AddModeCandidate, FINGERPRINT_LEN};

/// How long one probe's answer stands for its address. Shorter than
/// `INBOUND_HELLO_FRESH`: the device this one does not dial lists it only
/// from the hellos these probes deliver.
const PROBE_FRESH: Duration = Duration::from_secs(20);

/// How long an inbound hello stands for a device in the candidate list.
/// The prober repeats its pass every few seconds while it is in add-mode.
const INBOUND_HELLO_FRESH: Duration = Duration::from_secs(60);

/// A device that probed this one in add-mode: its hello, and when.
struct InboundHello {
    hostname: String,
    endpoint_id: String,
    heard_at: Instant,
}

/// One probe's answer, and when it came.
struct ProbedAt {
    candidate: AddModeCandidate,
    at: Instant,
}

/// Who is around, by what each device told us and where we last heard it.
#[derive(Default)]
pub struct PeerDirectory {
    /// The iroh key each candidate's hello reported, by device id: what a
    /// pairing leg to that candidate dials.
    keys: Mutex<HashMap<String, String>>,
    /// The newest address each known device was heard advertising from, by
    /// device id. Android restarts its advertisement with a fresh private
    /// address on every add-mode change, so the address a picker row was
    /// built from can be gone by the time the person presses Pair.
    addresses: Mutex<HashMap<String, String>>,
    /// Probe answers by address, so an advertiser already probed is not
    /// dialled again on every pass: each probe is a connection that can
    /// cross the other device's own dial to this one.
    probed: Mutex<HashMap<String, ProbedAt>>,
    /// Hellos from devices that dialled this one, by device id. Of two
    /// devices in add-mode only the lower fingerprint dials, so the other
    /// learns of it only from this.
    hellos: Mutex<HashMap<String, InboundHello>>,
}

impl PeerDirectory {
    /// The iroh key `device_id` reported, if it has been heard from.
    pub fn key(&self, device_id: &str) -> Option<String> {
        self.keys.lock().ok()?.get(device_id).cloned()
    }

    /// Where `device_id` was last heard advertising, if anywhere.
    pub fn latest_address(&self, device_id: &str) -> Option<String> {
        self.addresses.lock().ok()?.get(device_id).cloned()
    }

    /// Records `address` for whichever known device advertises
    /// `fingerprint`.
    pub fn note_advertiser(&self, address: &str, fingerprint: [u8; FINGERPRINT_LEN]) {
        let known: Vec<String> = self.keys.lock().map(|keys| keys.keys().cloned().collect()).unwrap_or_default();
        if let Some(device_id) = known.into_iter().find(|id| fingerprint_of(id) == fingerprint) {
            if let Ok(mut addresses) = self.addresses.lock() {
                addresses.insert(device_id, address.to_string());
            }
        }
    }

    /// The candidate a recent probe of `address` found, if one is fresh.
    pub fn recent_probe(&self, address: &str) -> Option<AddModeCandidate> {
        let probed = self.probed.lock().ok()?;
        probed.get(address).filter(|entry| entry.at.elapsed() < PROBE_FRESH).map(|entry| entry.candidate.clone())
    }

    /// Records what a probe of `candidate.address` answered.
    pub fn note_probe(&self, candidate: &AddModeCandidate, endpoint_id: String) {
        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(candidate.device_id.clone(), endpoint_id);
        }
        if let Ok(mut addresses) = self.addresses.lock() {
            addresses.insert(candidate.device_id.clone(), candidate.address.clone());
        }
        if let Ok(mut probed) = self.probed.lock() {
            probed.insert(candidate.address.clone(), ProbedAt { candidate: candidate.clone(), at: Instant::now() });
        }
    }

    /// Records the identity a prober sent.
    pub fn note_inbound_hello(&self, device_id: String, hostname: String, endpoint_id: String) {
        if let Ok(mut hellos) = self.hellos.lock() {
            hellos.insert(device_id, InboundHello { hostname, endpoint_id, heard_at: Instant::now() });
        }
    }

    /// The device that recently probed this one and advertises
    /// `fingerprint`, listed at `address` (where it advertises from, the
    /// address a pairing leg from this side can reach). Its key is recorded
    /// for that leg.
    pub fn prober_at(
        &self, address: &str, fingerprint: [u8; FINGERPRINT_LEN], my_device_id: &str,
    ) -> Option<AddModeCandidate> {
        let (device_id, hostname, endpoint_id) = {
            let hellos = self.hellos.lock().ok()?;
            let (device_id, hello) = hellos.iter().find(|(device_id, hello)| {
                fingerprint_of(device_id) == fingerprint && hello.heard_at.elapsed() < INBOUND_HELLO_FRESH
            })?;
            (device_id.clone(), hello.hostname.clone(), hello.endpoint_id.clone())
        };
        if device_id == my_device_id {
            return None;
        }
        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(device_id.clone(), endpoint_id);
        }
        Some(AddModeCandidate { address: address.to_string(), device_id, hostname })
    }
}
