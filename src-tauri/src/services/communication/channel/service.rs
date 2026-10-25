//! What every channel can do: the `ChannelService` trait.
//!
//! Each channel is one object that the `DeviceConnectionState` owns:
//! `network::NetworkChannel` and `bluetooth::BluetoothChannel`. One of each
//! for the whole app, each serving every paired device at once. A phone
//! paired with B, C and D — network to B and D, Bluetooth to B and C — has
//! two channels between them holding four sessions, not four channels.
//!
//! What a channel owns, and what it does not:
//!
//! | | |
//! |---|---|
//! | find · dial · accept · retry · registry · send | the channel |
//! | why this channel cannot reach a peer | the channel |
//! | the session loop and outbox drain | the channel owns it; the body is shared by default |
//! | authenticating a peer, and the pairing rules | one shared copy, in `pairing` |
//!
//! The last row is the load-bearing one. A channel *initiates* the handshake
//! — it is holding the `DataLink`, so nothing else can — but it does not
//! contain the rules of who is allowed in. Those live once, in `pairing`,
//! because a second channel must not be able to answer "is this device
//! paired with us" differently from the first.
//!
//! Adding a channel is then one `impl` of this trait plus a `channel_kinds`
//! row. See `../README.md` and `docs/glossary.md`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::services::communication::pairing::{ChannelKind, DeviceConnectionState};
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::pairing::ChannelProblem;

/// Everything one channel can do. Implemented once per `ChannelKind`.
///
/// A channel owns its own data. What it needs but does not own -- the
/// shared `DeviceConnectionState`, with the sessions, the presence records
/// and the database path -- is given to each call that needs it. The state
/// owns the channels, so a channel that kept a copy of the state would keep
/// itself alive.
#[async_trait]
pub trait ChannelService: Send + Sync {
    /// The kind of this channel.
    fn kind(&self) -> ChannelKind;

    /// Whether the pair with `peer_device_id` has this channel and it is on
    /// now: the stored state of the pair's channel, read from the database
    /// each time. A removed pair, a removed channel, and a channel that is
    /// off all give `false`.
    fn is_on(&self, state: &DeviceConnectionState, peer_device_id: &str) -> bool {
        tokio::task::block_in_place(|| {
            let mut conn = crate::services::db::open_db_at_path(&state.db_path);
            crate::services::communication::pairing::channels::is_enabled(&mut conn, peer_device_id, self.kind())
        })
    }

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
    /// never on a polling path — the passive signal behind `problem` covers
    /// the rest of the time.
    #[cfg(any(feature = "ui-plane", test))]
    async fn probe(&self, _state: &DeviceConnectionState) -> bool {
        self.available()
    }

    /// Start looking for peers: this channel's own way of noticing that
    /// another device is there. iroh's mDNS lookup for Network, scanning
    /// for Fini's service UUID over Bluetooth.
    ///
    /// Not gated on `ui-plane`, unlike `start_serving`: `cli-plane` dials out
    /// for sync, and it cannot dial what it has not found.
    fn start_discovery(&self, _state: &DeviceConnectionState) {}

    /// Begin serving: start whatever accept loop this channel needs, so a
    /// peer dialling in can be answered. Called once at startup.
    ///
    /// A no-op outside `ui-plane`: `cli-plane` dials out for sync but runs
    /// no inbound acceptor, so every channel's server is compiled out there.
    #[cfg(any(feature = "ui-plane", test))]
    fn start_serving(&self, _state: &DeviceConnectionState) {}

    /// Whether this channel holds a live, TLS-proven connection with a
    /// session to the peer right now -- green on the row (2026-10-13: green
    /// is a live connection, not a device heard).
    fn is_green(&self, state: &DeviceConnectionState, peer_device_id: &str) -> bool;

    /// A link to a paired device for the frames before a session
    /// (`PEER_ALPN`), such as a channel setup's `AddChannel`: this channel
    /// finds the device, for up to `within`, and connects to the key pinned
    /// for the pair. The channel does not know what is sent on it.
    #[cfg(any(feature = "ui-plane", test))]
    async fn connect_plain(
        &self, state: &DeviceConnectionState, peer_device_id: &str, within: std::time::Duration,
    ) -> Result<Box<dyn crate::services::communication::channel::DataLink>, String>;

    /// Whether this device is connecting to the peer on this channel right
    /// now -- for the gate to settle two connections that cross.
    #[cfg(any(feature = "ui-plane", test))]
    fn connecting(&self, _peer_device_id: &str) -> bool {
        false
    }

    /// Forget earlier failed attempts with this peer, so the next exchange
    /// is tried at once. For a person switching the channel on.
    #[cfg(any(feature = "ui-plane", test))]
    fn forget_failures(&self, _peer_device_id: &str) {}

    /// A problem on this device's side of the channel, if any (ADR-0008
    /// D6, D19) -- orange on the row, explained behind ⓘ.
    #[cfg(any(feature = "ui-plane", test))]
    fn problem(&self, state: &DeviceConnectionState) -> Option<ChannelProblem>;

    /// Keep this channel able to answer a peer, on every tick. A no-op where
    /// the accept loop was started once at startup.
    fn keep_serving(&self, _state: &DeviceConnectionState) {}
}

impl DeviceConnectionState {
    /// This state's channels, one per kind.
    pub fn channels(&self) -> [Arc<dyn ChannelService>; 2] {
        [self.network.clone(), self.bluetooth.clone()]
    }

    /// The channel of one kind, for callers that already know which channel
    /// they mean — a switch being flipped, a row being explained.
    pub fn channel(&self, kind: ChannelKind) -> Arc<dyn ChannelService> {
        match kind {
            ChannelKind::Network => self.network.clone(),
            ChannelKind::Bluetooth => self.bluetooth.clone(),
        }
    }
}
