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

use std::sync::Arc;

use async_trait::async_trait;

use super::radio::{for_this_device, Radio};
use crate::services::communication::pairing::{ChannelKind, DeviceConnectionState};
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::pairing::ChannelProblem;

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
    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool;

    /// Whether this channel can be switched on right now (ADR-0008 D6): the
    /// platform supports it and, as far as the passive signals know, the
    /// hardware is working.
    #[cfg(any(feature = "ui-plane", test))]
    fn usable(&self) -> bool {
        self.available()
    }

    /// Ask the hardware directly, right now.
    ///
    /// Separate from `available` because the answer can change while the app
    /// runs: a radio switched off in the OS is not a platform fact. Costly
    /// enough that it is called when a person flips a switch and is watching,
    /// never on a polling path — the passive signal behind `why_not` covers
    /// the rest of the time.
    #[cfg(any(feature = "ui-plane", test))]
    async fn probe(&self) -> bool {
        self.available()
    }

    /// Start looking for peers: this channel's own way of noticing that
    /// another device is there. mDNS and UDP beacons for Network, scanning
    /// for Fini's service UUID over Bluetooth.
    ///
    /// Not gated on `ui-plane`, unlike `start_serving`: `cli-plane` dials out
    /// for sync, and it cannot dial what it has not found.
    fn start_discovery(&self) {}

    /// Begin serving: start whatever accept loop this channel needs, so a
    /// peer dialling in can be answered. Called once at startup.
    ///
    /// A no-op outside `ui-plane`: `cli-plane` dials out for sync but runs
    /// no inbound acceptor, so every channel's server is compiled out there.
    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self) {}

    /// Start an exchange with this peer unless one is running (ADR-0008
    /// D10): straight away when the peer is present, otherwise through this
    /// channel's delivery search (D12). Called when there is work for it.
    fn request_exchange(&self, peer_device_id: &str);

    /// Run the status search while `active`: someone is looking at the
    /// channel rows, so presence must stay current (ADR-0008 D12, D13).
    /// Delivery and setup searches are asked for by `request_exchange` and
    /// the setup flow; each channel merges all of them into one search of
    /// its own kind (D18). A no-op for a channel whose presence signal is
    /// always heard, like Network's beacon.
    #[cfg(any(feature = "ui-plane", test))]
    fn watch_presence(&self, _active: bool) {}

    /// Whether the peer was seen on this channel within its channel timeout
    /// (ADR-0008 D9) -- green on the row.
    fn is_present(&self, peer_device_id: &str) -> bool;

    /// A problem on this device's side of the channel, if any (ADR-0008
    /// D6, D19) -- orange on the row, explained behind ⓘ.
    #[cfg(any(feature = "ui-plane", test))]
    fn problem(&self) -> Option<ChannelProblem>;

    /// Keep this channel able to answer a peer, on every tick. A no-op where
    /// the accept loop was started once at startup.
    fn keep_serving(&self) {}
}

/// The channel services for one `DeviceConnectionState`. One per kind.
///
/// Not one per pair: a single `NetworkChannelService` serves every paired
/// device that has the Network channel on. In a running app there is exactly
/// one `DeviceConnectionState`, so there is exactly one of each of these.
///
/// Deliberately **not** cached in a process-wide `OnceLock`. That would bind
/// every later caller to whichever state happened to construct it first,
/// which is invisible in the app (there is only one) and wrong everywhere
/// else — a test would silently get a service pointing at a previous test's
/// database. What the services hold today is a `DeviceConnectionState` clone
/// and a radio, so rebuilding them per call costs an `Arc` bump. That stops
/// being true in the step that moves the session registry inside them, and
/// they will then be owned by the state rather than rebuilt.
pub fn services(state: &DeviceConnectionState) -> Vec<Arc<dyn ChannelService>> {
    vec![
        Arc::new(NetworkChannelService::new(state.clone())),
        Arc::new(BluetoothChannelService::new(state.clone(), for_this_device())),
    ]
}

/// The service for one kind, for callers that already know which channel
/// they mean — a switch being flipped, a row being explained.
pub fn service_for(state: &DeviceConnectionState, kind: ChannelKind) -> Arc<dyn ChannelService> {
    services(state)
        .into_iter()
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
    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool {
        true
    }

    fn start_discovery(&self) {
        self.state.start_network_discovery();
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self) {
        tauri::async_runtime::spawn(super::tcp_ws::run_server(
            self.state.clone(),
            self.state.db_path.clone(),
        ));
    }

    fn request_exchange(&self, peer_device_id: &str) {
        super::tcp_ws::start_exchange(&self.state, peer_device_id);
    }

    fn is_present(&self, peer_device_id: &str) -> bool {
        self.state.network_peer_available(peer_device_id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn problem(&self) -> Option<ChannelProblem> {
        self.state
            .network_broadcast_failing()
            .then_some(ChannelProblem::NetworkUnavailable)
    }
}

/// The Bluetooth channel: peers found by scanning for Fini's service UUID,
/// connected over GATT.
pub struct BluetoothChannelService {
    state: DeviceConnectionState,
    /// Injected, not reached for: this service is the policy, and the radio
    /// is the mechanism. Real hardware on a device, loopback on CI, and the
    /// service cannot tell the difference — which is what makes the CI lane
    /// worth anything.
    radio: Box<dyn Radio>,
}

impl BluetoothChannelService {
    pub fn new(state: DeviceConnectionState, radio: Box<dyn Radio>) -> Self {
        Self { state, radio }
    }
}


#[async_trait]
impl ChannelService for BluetoothChannelService {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool {
        self.radio.available()
    }

    #[cfg(any(feature = "ui-plane", test))]
    async fn probe(&self) -> bool {
        self.radio.probe().await
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn usable(&self) -> bool {
        self.radio.available() && self.radio.adapter_available()
    }

    fn start_discovery(&self) {
        self.radio.start_discovery(&self.state);
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self) {
        self.radio.serve(&self.state);
    }

    fn request_exchange(&self, peer_device_id: &str) {
        self.radio.request_exchange(&self.state, peer_device_id);
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn watch_presence(&self, active: bool) {
        self.radio.watch_presence(&self.state, active);
    }

    fn is_present(&self, peer_device_id: &str) -> bool {
        self.radio.is_reachable(peer_device_id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn problem(&self) -> Option<ChannelProblem> {
        if !self.radio.available() {
            Some(ChannelProblem::BluetoothNotSupported)
        } else if !self.radio.adapter_available() {
            Some(ChannelProblem::BluetoothUnavailable)
        } else {
            None
        }
    }

    fn keep_serving(&self) {
        self.radio.keep_serving(&self.state);
    }
}
