//! How the Bluetooth channel actually reaches a peer.
//!
//! `BluetoothChannelService` is the policy — when to dial, what to report,
//! which peers are candidates. A `Radio` is the mechanism underneath it, and
//! the service takes one at construction rather than reaching for a module:
//! that is the seam that lets the same service run against real hardware and
//! against no hardware at all.
//!
//! | | reaches a peer by | used where |
//! |---|---|---|
//! | `GattRadio` | BLE advertising, scanning and GATT, via `ble-gatt` | real devices |
//! | `LoopbackRadio` | a TCP connection to `127.0.0.1` | CI, which has no radio |
//!
//! `LoopbackRadio` is what used to be called "Sim", and the rename is the
//! point: it was read as a fourth kind of channel sitting beside Network and
//! Bluetooth, which it never was. A person cannot choose it, it appears in no
//! table and no screen, and it has no discovery of its own — peers are a list
//! of port numbers in an environment variable. It is one implementation of
//! the Bluetooth channel, chosen when the machine has no Bluetooth.
//!
//! It earns its place because CI genuinely has no radio, and the behaviour
//! most worth testing is "the network is unavailable, so use the other
//! channel". `ble-gatt`'s mock broker covers a different and more faithful
//! case — all the real BLE code, with only the radio faked — and the two
//! lanes prove different things.

use async_trait::async_trait;

use crate::services::communication::pairing::DeviceConnectionState;

/// The mechanism the Bluetooth channel uses. One implementation per way of
/// reaching a peer without a network.
#[async_trait]
pub trait Radio: Send + Sync {
    /// Whether this radio can work on this device at all — a platform fact.
    fn available(&self) -> bool;

    /// Ask the hardware directly, right now. Distinct from `available`
    /// because a radio switched off in the OS is not a platform fact.
    async fn probe(&self) -> bool {
        self.available()
    }

    /// Start looking for peers.
    ///
    /// Scanning over a real radio is driven by the dial loop rather than a
    /// standing worker, so `GattRadio` has nothing to start here — but the
    /// method exists because "how this channel finds peers" is a question a
    /// radio must be able to answer, not one the caller should have to know
    /// the shape of.
    fn start_discovery(&self, state: &DeviceConnectionState) {
        let _ = state;
    }

    /// Start accepting inbound connections.
    fn serve(&self, state: &DeviceConnectionState);

    /// Dial the peers in `peers` that have no session yet.
    fn dial(&self, state: &DeviceConnectionState, peers: &[String]);

    /// Whether this peer has been heard from recently enough to be worth
    /// dialling, as far as this radio can tell without trying.
    fn is_reachable(&self, peer_device_id: &str) -> bool;

    /// Whether the hardware worked the last time it was asked to do
    /// anything, from whatever the background loops already observed.
    ///
    /// The passive counterpart to `probe`: free to read, and therefore the
    /// one a status poll uses. `true` when nothing has been attempted yet,
    /// so an untried radio is never accused of being off.
    fn adapter_available(&self) -> bool {
        true
    }

    /// Whether this peer's automatic dial attempts have given up.
    fn dial_exhausted(&self, peer_device_id: &str) -> bool {
        let _ = peer_device_id;
        false
    }

    /// Give this peer a fresh retry window after its attempts gave up.
    fn retry_now(&self, state: &DeviceConnectionState, peer_device_id: &str) {
        let _ = (state, peer_device_id);
    }
}

/// The real one: BLE advertising, scanning and GATT through `ble-gatt`.
pub struct GattRadio;

#[async_trait]
impl Radio for GattRadio {
    fn available(&self) -> bool {
        cfg!(any(target_os = "linux", target_os = "android"))
    }

    async fn probe(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            super::ble::probe_adapter_available().await
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            // No adapter on this platform at all, so there is no radio to be
            // off. Answering `false` would blame the person's hardware for a
            // platform decision.
            true
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn serve(&self, state: &DeviceConnectionState) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        tauri::async_runtime::spawn(super::ble::run_server(
            state.clone(),
            state.db_path.clone(),
        ));
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = state;
    }

    #[cfg(not(any(feature = "ui-plane", test)))]
    fn serve(&self, _state: &DeviceConnectionState) {}

    fn dial(&self, state: &DeviceConnectionState, peers: &[String]) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            // Android starts advertising lazily, from the first real tick
            // rather than at setup: the Activity context it needs does not
            // exist when the app boots (see `ble::start_peripheral_once`).
            // Linux starts its peripheral role from `serve`, like any other
            // accept loop.
            //
            // Gated on the Nearby-devices permission actually being held,
            // because this runs from a background tick rather than a user
            // action. Without the gate the first tick after install starts
            // advertising, Android throws SecurityException out of
            // `startAdvertising`, and the loop retries every 60s forever --
            // work nobody asked for, failing invisibly. The check must sit
            // *outside* `start_peripheral_once`, whose `Once` would be spent
            // by the first ungranted attempt and never retried afterwards.
            #[cfg(target_os = "android")]
            if crate::services::android_context::call_static_context_to_bool(
                "com.fini.app.BluetoothPairing",
                "hasPermissions",
            ) {
                super::ble::start_peripheral_once(state.clone(), state.db_path.clone());
            }

            super::ble::spawn_dial_loop(state, state.db_path.clone(), peers);
            // The dialling side's own give-up timer has no equivalent on
            // whichever side of a pair never dials.
            super::ble::check_accepting_side_exhaustion(state, peers);
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = (state, peers);
    }

    fn is_reachable(&self, peer_device_id: &str) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            super::ble::peer_seen_advertising_recently(peer_device_id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = peer_device_id;
            false
        }
    }

    fn adapter_available(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            !super::ble::is_bluetooth_adapter_unavailable()
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            true
        }
    }

    fn dial_exhausted(&self, peer_device_id: &str) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            super::ble::is_bluetooth_dial_exhausted(peer_device_id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = peer_device_id;
            false
        }
    }

    fn retry_now(&self, state: &DeviceConnectionState, peer_device_id: &str) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        super::ble::retry_bluetooth_dial(state, peer_device_id);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = (state, peer_device_id);
    }
}

/// The one for machines with no Bluetooth: a TCP connection to `127.0.0.1`,
/// with peers given as a list of ports rather than discovered.
pub struct LoopbackRadio;

#[async_trait]
impl Radio for LoopbackRadio {
    /// Always. It is loopback — if the process is running, it works.
    fn available(&self) -> bool {
        true
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn serve(&self, state: &DeviceConnectionState) {
        super::loopback::maybe_spawn_server(state.clone(), state.db_path.clone());
    }

    #[cfg(not(any(feature = "ui-plane", test)))]
    fn serve(&self, _state: &DeviceConnectionState) {}

    fn dial(&self, state: &DeviceConnectionState, peers: &[String]) {
        let peers: std::collections::HashSet<String> = peers.iter().cloned().collect();
        super::loopback::spawn_fallback_dial_loop(state, state.db_path.clone(), &peers);
    }

    /// No discovery to consult: a configured port either answers or it does
    /// not, and finding that out is the dial. Saying "reachable" here lets
    /// the dial be the test, which is what this radio is for.
    fn is_reachable(&self, _peer_device_id: &str) -> bool {
        true
    }
}

/// Which radio this process should use.
///
/// `FINI_LOOPBACK_PORT` being set is the injection point: nothing but a test
/// harness sets it, and a build that has it set has, by construction, no
/// radio to use instead.
pub fn for_this_device() -> Box<dyn Radio> {
    if super::loopback::configured_listen_port().is_some() {
        Box::new(LoopbackRadio)
    } else {
        Box::new(GattRadio)
    }
}
