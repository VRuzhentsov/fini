//! One object per channel, owning everything that channel needs to reach a
//! peer.
//!
//! A `ChannelService` is a **singleton**: one `NetworkChannelService` and one
//! `BluetoothChannelService` for the whole app, each serving every paired
//! device at once. A phone paired with B, C and D — network to B and D,
//! Bluetooth to B and C — has two services between them holding four
//! sessions, not four services.
//!
//! What a service owns, and what it does not:
//!
//! | | |
//! |---|---|
//! | find · dial · accept · retry · registry · send | the service |
//! | why this channel cannot reach a peer | the service |
//! | the session loop and outbox drain | the service owns it; the body is shared by default |
//! | authenticating a peer, and the pairing rules | one shared copy, in `pairing` |
//!
//! The last row is the load-bearing one. A service *initiates* the handshake
//! — it is holding the `DataLink`, so nothing else can — but it does not
//! contain the rules of who is allowed in. Those live once, in `pairing`,
//! because a second channel must not be able to answer "is this device
//! paired with us" differently from the first.
//!
//! Adding a channel is then one `impl` of this trait plus a `channel_kinds`
//! row. See `../README.md` and `docs/glossary.md`.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::services::communication::pairing::{ChannelKind, DeviceConnectionState};

/// Everything one channel can do. Implemented once per `ChannelKind`.
///
/// Methods take `&self` alone: a service is constructed with the app state
/// and database path it needs, so callers name a peer rather than passing
/// the world in on every call.
#[async_trait]
pub trait ChannelService: Send + Sync {
    fn kind(&self) -> ChannelKind;

    /// Whether this channel can work on this device *at all* — a platform
    /// fact, not a per-pair one. False means no build of this app on this OS
    /// can use it, so the Device page must never offer it as something a
    /// person could switch on and wait for.
    fn available(&self) -> bool;

    /// Ask the hardware directly, right now.
    ///
    /// Separate from `available` because the answer can change while the app
    /// runs: a radio switched off in the OS is not a platform fact. Costly
    /// enough that it is called when a person flips a switch and is watching,
    /// never on a polling path — the passive signal behind `why_not` covers
    /// the rest of the time.
    async fn probe(&self) -> bool {
        self.available()
    }

    /// Begin serving: start whatever accept loop this channel needs, so a
    /// peer dialling in can be answered. Called once at startup.
    ///
    /// A no-op outside `ui-plane`: `cli-plane` dials out for sync but runs
    /// no inbound acceptor, so every channel's server is compiled out there.
    fn start_serving(&self) {}

    /// Dial every peer in `peers` that this channel could reach and does not
    /// already have a session with. Called on each sync tick.
    ///
    /// Takes the whole set rather than one peer because that is the shape
    /// every channel's dial loop actually has today — it needs to know which
    /// peers are *not* worth trying as much as which are.
    fn start_dialing(&self, peers: &[String]);

    /// Whether this channel could reach this peer right now, as far as it
    /// can tell without trying. A peer's beacon arriving, or its
    /// advertisement being heard recently.
    fn is_reachable(&self, peer_device_id: &str) -> bool;

    /// Give this peer a fresh retry window after its automatic attempts gave
    /// up. A no-op where the channel has no backoff to clear.
    fn retry_now(&self, peer_device_id: &str) {
        let _ = peer_device_id;
    }
}

/// The app's channel services. One instance of each kind for the whole
/// process, built on first use and shared from then on.
///
/// Not one per pair: a single `NetworkChannelService` serves every paired
/// device that has the Network channel on.
pub fn services(state: &DeviceConnectionState) -> &'static [Arc<dyn ChannelService>] {
    static SERVICES: OnceLock<Vec<Arc<dyn ChannelService>>> = OnceLock::new();
    SERVICES.get_or_init(|| {
        vec![
            Arc::new(NetworkChannelService::new(state.clone())),
            Arc::new(BluetoothChannelService::new(state.clone())),
        ]
    })
}

/// The service for one kind, for callers that already know which channel
/// they mean — a switch being flipped, a row being explained.
pub fn service_for(
    state: &DeviceConnectionState,
    kind: ChannelKind,
) -> &'static Arc<dyn ChannelService> {
    services(state)
        .iter()
        .find(|service| service.kind() == kind)
        .expect("every ChannelKind has a service")
}

/// The Network channel: peers found by mDNS/UDP presence, connected over
/// TCP-WS.
pub struct NetworkChannelService {
    state: DeviceConnectionState,
}

impl NetworkChannelService {
    pub fn new(state: DeviceConnectionState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ChannelService for NetworkChannelService {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Network
    }

    /// Every platform Fini runs on has a network stack. Whether a *peer* is
    /// reachable over it is `is_reachable`'s question, not this one.
    fn available(&self) -> bool {
        true
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self) {
        tauri::async_runtime::spawn(super::tcp_ws::run_server(
            self.state.clone(),
            self.state.db_path.clone(),
        ));
    }

    fn start_dialing(&self, peers: &[String]) {
        let peers: std::collections::HashSet<String> = peers.iter().cloned().collect();
        super::tcp_ws::spawn_dial_loop(&self.state, self.state.db_path.clone(), &peers);
    }

    fn is_reachable(&self, peer_device_id: &str) -> bool {
        self.state.network_peer_available(peer_device_id)
    }

    /// Nothing to clear: the network dial loop backs off per attempt and
    /// never gives up on a peer outright, so there is no exhausted state a
    /// person could be stuck behind.
    fn retry_now(&self, _peer_device_id: &str) {
        crate::services::communication::sync::commands::notify_sync_work_pending();
    }
}

/// The Bluetooth channel: peers found by scanning for Fini's service UUID,
/// connected over GATT.
pub struct BluetoothChannelService {
    state: DeviceConnectionState,
}

impl BluetoothChannelService {
    pub fn new(state: DeviceConnectionState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ChannelService for BluetoothChannelService {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    /// Only where an adapter is wired up at all. Everywhere else the Device
    /// page says so plainly rather than offering a channel that can never
    /// start.
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
    fn start_serving(&self) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        tauri::async_runtime::spawn(super::ble::run_server(
            self.state.clone(),
            self.state.db_path.clone(),
        ));
    }

    fn start_dialing(&self, peers: &[String]) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            // Android starts advertising lazily, from the first real tick
            // rather than at setup: the Activity context it needs does not
            // exist when the app boots (see `ble::start_peripheral_once`).
            // Linux starts its peripheral role from `start_serving`, like any
            // other accept loop.
            //
            // Gated on the Nearby-devices permission actually being held,
            // because this runs from a background tick rather than a user
            // action. Without the gate the first tick after install starts
            // advertising, Android throws SecurityException out of
            // `startAdvertising`, and the loop retries it every 60s forever --
            // work nobody asked for, failing invisibly. The check must sit
            // *outside* `start_peripheral_once`, whose `Once` would be spent
            // by the first ungranted attempt and never retried after the
            // person says yes.
            #[cfg(target_os = "android")]
            if crate::services::android_context::call_static_context_to_bool(
                "com.fini.app.BluetoothPairing",
                "hasPermissions",
            ) {
                super::ble::start_peripheral_once(self.state.clone(), self.state.db_path.clone());
            }

            super::ble::spawn_dial_loop(&self.state, self.state.db_path.clone(), peers);
            // The dialling side's own give-up timer has no equivalent on
            // whichever side of a pair never dials.
            super::ble::check_accepting_side_exhaustion(&self.state, peers);
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = peers;
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

    fn retry_now(&self, peer_device_id: &str) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        super::ble::retry_bluetooth_dial(&self.state, peer_device_id);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = peer_device_id;
    }
}
