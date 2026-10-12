//! What this device advertises over Bluetooth, and whether it advertises
//! at all.
//!
//! One `Advertiser` per `DeviceConnectionState`, so two devices in one test
//! process each advertise their own identity and add-mode.

#[cfg(any(feature = "ui-plane", test))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
#[cfg(any(feature = "ui-plane", test))]
use std::time::Duration;
use std::time::Instant;

use tokio::sync::watch;

#[cfg(any(feature = "ui-plane", test))]
use super::{datagram_config, fingerprint_of, ADD_MODE_FLAG_BYTE, FINGERPRINT_LEN, FINI_MANUFACTURER_ID};
#[cfg(any(feature = "ui-plane", test))]
use ble_gatt::datagram::DatagramConfig;
use crate::services::communication::pairing::DeviceIdentity;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::sync::types::PeerFrame;

/// How long this device stays reachable after a Bluetooth init completes
/// here. Its ack to the peer's hello can be lost when the link drops, so the
/// peer may still be searching; the gate still answers it (ADR-0008 D2), but
/// only if the peer can find this device.
#[cfg(any(feature = "ui-plane", test))]
const ANSWER_AFTER_SETUP: Duration = Duration::from_secs(120);

pub struct Advertiser {
    /// This device's advertised identity fingerprint: what lets a scanning
    /// peer tell this device apart from any other Fini install without
    /// connecting to it first.
    #[cfg(any(feature = "ui-plane", test))]
    fingerprint: [u8; FINGERPRINT_LEN],
    /// This device's `DiscoveryHello`, carrying its identity.
    #[cfg(any(feature = "ui-plane", test))]
    hello: PeerFrame,
    /// Whether this device is in add-mode, watched by `run_server`'s
    /// peripheral loop so a toggle can trigger a fresh advertisement
    /// carrying (or dropping) the add-mode flag without restarting the whole
    /// peripheral task -- Android's `start_peripheral_once` is deliberately a
    /// one-time start (ADR-0002 in ble-gatt: constructing `AndroidBackend`
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
    /// only that one caller. `GattRadio::serve` spawned `run_server` directly
    /// as well, so on Android both ran: two accept loops, both handed the
    /// same inbound central, both running the gate on it. One claimed the
    /// session, the other was rejected as a duplicate, and dropping the
    /// rejected link released the session the winner was using.
    #[cfg(any(feature = "ui-plane", test))]
    peripheral_running: AtomicBool,
}

impl Advertiser {
    pub fn new(identity: &DeviceIdentity) -> Self {
        #[cfg(not(any(feature = "ui-plane", test)))]
        let _ = identity;
        Self {
            #[cfg(any(feature = "ui-plane", test))]
            fingerprint: fingerprint_of(&identity.device_id),
            #[cfg(any(feature = "ui-plane", test))]
            hello: PeerFrame::DiscoveryHello {
                device_id: identity.device_id.clone(),
                hostname: identity.hostname.clone(),
                endpoint_id: identity.endpoint_id.clone(),
            },
            add_mode: watch::channel(false).0,
            wanted: watch::channel(false).0,
            answer_after_setup_until: StdMutex::new(None),
            #[cfg(any(feature = "ui-plane", test))]
            peripheral_running: AtomicBool::new(false),
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn hello(&self) -> &PeerFrame {
        &self.hello
    }

    /// The datagram config `run_server` advertises: Fini's service plus one
    /// flags byte and the identity fingerprint as manufacturer data.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn advertised_config(&self) -> DatagramConfig {
        let mut config = datagram_config();
        let mut payload = Vec::with_capacity(1 + FINGERPRINT_LEN);
        payload.push(if self.in_add_mode() { ADD_MODE_FLAG_BYTE } else { 0 });
        payload.extend_from_slice(&self.fingerprint);
        config.advertised_manufacturer_data.insert(FINI_MANUFACTURER_ID, payload);
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

    #[cfg(any(feature = "ui-plane", test))]
    pub fn watch_add_mode(&self) -> watch::Receiver<bool> {
        self.add_mode.subscribe()
    }

    #[cfg(any(feature = "ui-plane", test))]
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

    /// A Bluetooth init completed here: keep advertising for a while, then
    /// look again whether anything still wants it.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn keep_answering_after_setup(&self) {
        if let Ok(mut until) = self.answer_after_setup_until.lock() {
            *until = Some(Instant::now() + ANSWER_AFTER_SETUP);
        }
        crate::services::communication::sync::commands::notify_sync_work_pending_after(
            ANSWER_AFTER_SETUP + Duration::from_secs(1),
        );
    }

    /// Takes the peripheral role; false if an acceptor already runs.
    #[cfg(any(feature = "ui-plane", test))]
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

    fn identity(device_id: &str) -> DeviceIdentity {
        DeviceIdentity {
            device_id: device_id.to_string(),
            hostname: "host".to_string(),
            endpoint_id: "key".to_string(),
        }
    }

    /// The peripheral acceptor must be a singleton, whatever starts it.
    ///
    /// It was not: `GattRadio::serve` spawned `run_server` on Linux *and*
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
