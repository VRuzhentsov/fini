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
use crate::services::communication::pairing::{
    channel_status, ChannelKind, ChannelStatusCode, DeviceConnectionState,
};

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

    /// Whether this channel's automatic attempts at this peer have given up.
    ///
    /// Surfaced on its own, separate from `why_not`, because the row stays
    /// clickable in that state — it is the one reason a person can act on by
    /// asking for another try.
    fn dial_exhausted(&self, peer_device_id: &str) -> bool {
        let _ = peer_device_id;
        false
    }

    /// Why this channel cannot reach this peer, or `None` if nothing is in
    /// the way. What the row on the Device page says out loud.
    ///
    /// Only the channel can answer honestly. "Bluetooth is off on this
    /// computer" and "Pixel 8 isn't nearby" are claims about different
    /// machines, and only one of them is something the person can act on
    /// where they are standing — so the answer has to come from whichever
    /// channel actually knows, not from a caller guessing between them.
    ///
    /// `enabled` is the pair's own switch, passed in because it is a fact
    /// about the row rather than about the channel; where it sits in each
    /// channel's ordering is the channel's business.
    fn why_not(&self, peer_device_id: &str, enabled: bool) -> Option<ChannelStatusCode>;
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

    fn why_not(&self, peer_device_id: &str, enabled: bool) -> Option<ChannelStatusCode> {
        channel_status::network_unconfigured_code(enabled, self.is_reachable(peer_device_id))
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

    fn available(&self) -> bool {
        self.radio.available()
    }

    async fn probe(&self) -> bool {
        self.radio.probe().await
    }

    fn start_discovery(&self) {
        self.radio.start_discovery(&self.state);
    }

    fn start_serving(&self) {
        self.radio.serve(&self.state);
    }

    fn start_dialing(&self, peers: &[String]) {
        self.radio.dial(&self.state, peers);
    }

    fn is_reachable(&self, peer_device_id: &str) -> bool {
        self.radio.is_reachable(peer_device_id)
    }

    fn retry_now(&self, peer_device_id: &str) {
        self.radio.retry_now(&self.state, peer_device_id);
    }

    fn dial_exhausted(&self, peer_device_id: &str) -> bool {
        self.radio.dial_exhausted(peer_device_id)
    }

    fn why_not(&self, peer_device_id: &str, enabled: bool) -> Option<ChannelStatusCode> {
        channel_status::bluetooth_unconfigured_code(
            self.radio.available(),
            enabled,
            self.radio.adapter_available(),
            // A live session is the strongest evidence of nearness there is,
            // and it outranks the advertisement record entirely: scanning
            // stops while a session is up, so the last-seen stamp goes stale
            // and the row would report "not nearby" about a peer it is
            // actively talking to.
            self.state
                .has_session_on(peer_device_id, super::TransportKind::Bluetooth)
                || self.radio.is_reachable(peer_device_id),
            self.radio.dial_exhausted(peer_device_id),
        )
    }
}
