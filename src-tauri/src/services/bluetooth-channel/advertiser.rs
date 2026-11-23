//! What this device advertises over Bluetooth, and whether it advertises
//! at all.
//!
//! One `Advertiser` per Bluetooth channel, so two devices in one test
//! process each advertise their own identity and add-mode.

use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

use super::{
    datagram_config, fingerprint_of, ADD_MODE_FLAG_BYTE, FINGERPRINT_LEN, FINI_MANUFACTURER_ID,
};
use crate::models::Identity;
use ble_gatt::datagram::DatagramConfig;

/// The longest name the scan response carries on every platform: Android
/// fits 13 bytes next to the service UUID, Linux 29. Cut here, so both show
/// the same name.
const SHORT_NAME_MAX_BYTES: usize = 13;

/// `name` cut to `SHORT_NAME_MAX_BYTES` at a character boundary.
fn short_name(name: &str) -> String {
    let mut end = name.len().min(SHORT_NAME_MAX_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

pub struct Advertiser {
    /// This device's advertised identity fingerprint: what lets a scanning
    /// peer tell this device apart from any other Fini install without
    /// connecting to it first.
    fingerprint: [u8; FINGERPRINT_LEN],
    /// This device's name, cut to `SHORT_NAME_MAX_BYTES`, sent in the scan
    /// response so the Add Device list can show it without a connection.
    short_name: String,
    /// The readable metadata retains the complete application name.
    hostname: String,
    /// Add Device state, watched by the one advertisement task.
    /// A change updates the shared owner without replacing its server.
    add_mode: watch::Sender<bool>,
    /// Whether this device should advertise (ADR-0008 D8): while it has a
    /// Bluetooth channel on, is setting one up, or is being paired. Otherwise
    /// the radio stays quiet -- there is nobody it should be reachable by.
    wanted: watch::Sender<bool>,
    /// One peripheral acceptor, enforced where the loop actually runs rather
    /// than at each call site.
    ///
    /// This used to be a `Once` inside `start_peripheral_once`, which guarded
    /// only that one caller. `start_serving` spawned `run_server` directly
    /// as well, so on Android both ran: two accept loops, both handed the
    /// same inbound central, both running the gate on it. One claimed the
    /// session, the other was rejected as a duplicate, and dropping the
    /// rejected link released the session the winner was using.
    peripheral_running: AtomicBool,
}

impl Advertiser {
    pub fn new(identity: &Identity) -> Self {
        Self {
            fingerprint: fingerprint_of(&identity.device_id),
            short_name: short_name(&identity.hostname),
            hostname: identity.hostname.clone(),
            add_mode: watch::channel(false).0,
            wanted: watch::channel(false).0,
            peripheral_running: AtomicBool::new(false),
        }
    }

    fn advertisement(&self) -> ble_gatt::advertisement::GattAdvertisement {
        let config = self.advertised_config();
        ble_gatt::advertisement::GattAdvertisement {
            manufacturer_data: config.advertised_manufacturer_data,
            service_data: config.advertised_service_data,
            local_name: Some(self.hostname.clone()),
        }
    }

    /// An uncertain update needs a hide fence and a complete hidden update.
    pub(super) async fn update(
        &self,
        link: &ble_gatt::PeerLink,
        visible: bool,
        uncertain: &mut bool,
    ) -> Result<(), String> {
        if *uncertain || !visible {
            *uncertain = true;
            link.set_advertising(false)
                .await
                .map_err(|err| format!("advertisement hide failed: {err}"))?;
        }
        *uncertain = true;
        if let Err(err) = link.update_advertisement(self.advertisement()).await {
            // Show requires a complete hidden update after this fence.
            let fence = link.set_advertising(false).await;
            return Err(format!(
                "advertisement write failed: {err}; hide fence: {fence:?}"
            ));
        }
        if let Err(err) = link.set_advertising(visible).await {
            let fence = link.set_advertising(false).await;
            return Err(format!(
                "advertisement visibility failed: {err}; hide fence: {fence:?}"
            ));
        }
        *uncertain = false;
        Ok(())
    }

    /// The datagram config `run_server` advertises: Fini's service plus one
    /// flags byte and the identity fingerprint as manufacturer data, and the
    /// short name in the scan response.
    pub fn advertised_config(&self) -> DatagramConfig {
        let mut config = datagram_config();
        let mut payload = Vec::with_capacity(1 + FINGERPRINT_LEN);
        payload.push(if self.in_add_mode() {
            ADD_MODE_FLAG_BYTE
        } else {
            0
        });
        payload.extend_from_slice(&self.fingerprint);
        config
            .advertised_manufacturer_data
            .insert(FINI_MANUFACTURER_ID, payload);
        config.advertised_local_name = Some(self.short_name.clone());
        // The record travels in ble-gatt's fast advertisement, and the scan
        // gives it back under the same ID (ble-gatt ADR-0012 §2).
        config.fast_advertisement_company = Some(FINI_MANUFACTURER_ID);
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

    pub fn watch_add_mode(&self) -> watch::Receiver<bool> {
        self.add_mode.subscribe()
    }

    pub fn watch_wanted(&self) -> watch::Receiver<bool> {
        self.wanted.subscribe()
    }

    pub fn wanted(&self) -> bool {
        *self.wanted.borrow()
    }

    /// Records whether to advertise; see `refresh_advertising`.
    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.send_if_modified(|current| {
            let changed = *current != wanted;
            *current = wanted;
            changed
        });
    }

    /// Takes the peripheral role; false if an acceptor already runs.
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

    fn identity(device_id: &str) -> Identity {
        let secret_key = iroh::SecretKey::generate();
        Identity {
            device_id: device_id.to_string(),
            hostname: "host".to_string(),
            endpoint_id: secret_key.public().to_string(),
            secret_key,
        }
    }

    /// The peripheral acceptor must be a singleton, whatever starts it.
    ///
    /// It was not: `start_serving` spawned `run_server` on Linux *and*
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
        assert!(
            advertiser.claim_peripheral_role(),
            "the first start takes the role"
        );
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
        assert_eq!(
            payload[0] & ADD_MODE_FLAG_BYTE,
            0,
            "add-mode bit must be clear"
        );
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
        assert_eq!(
            &payload[1..],
            &expected,
            "fingerprint is unchanged by add-mode"
        );

        advertiser.set_add_mode(false);
        let disabled_again = advertiser.advertised_config();
        assert_eq!(
            disabled_again
                .advertised_manufacturer_data
                .get(&FINI_MANUFACTURER_ID)
                .map(|p| p[0] & ADD_MODE_FLAG_BYTE),
            Some(0),
            "must stop signalling add-mode once it is left again"
        );
    }
}
