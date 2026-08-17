use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::services::device_connection::types::{
    PairAcceptPayload, PairCompletePayload, PairRequestPayload,
};
use crate::services::device_connection::CustomSpaceDescriptor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEventEnvelope {
    pub event_id: String,
    pub correlation_id: String,
    pub origin_device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub op_type: String,
    pub payload: Option<String>,
    pub updated_at: String,
    pub created_at: String,
}

/// What can be sent through a claimed session's mailbox (`run_session`'s
/// `rx`): forward a frame to the peer over the wire. ADR-0003 revision:
/// the old `Close` variant (a manual transport switch force-closing the
/// non-preferred session) no longer has a reason to exist -- both
/// transports stay connected regardless of which is primary, so a pin
/// change just relabels which one is primary; nothing needs closing.
#[derive(Debug)]
pub enum SessionCommand {
    Forward(PeerFrame),
}

pub type SessionSender = mpsc::Sender<SessionCommand>;

/// A message of the transport-neutral Fini peer protocol: pairing handshake
/// plus authenticated sync. Carried by whichever `Transport`/`Link` is
/// currently selected for a peer (see `crate::services::transport`).
/// Bump whenever a new `PeerFrame` variant is introduced that must not be
/// *proactively* sent to a peer that might not understand it yet (unlike a
/// frame sent only in reply to something the peer itself sent first, which
/// proves they're already on a compatible build). `PeerFrame::Unknown`
/// alone only protects an updated build's own deserialization of frames
/// *it* receives -- it does nothing for an older, already-installed peer
/// receiving a frame kind its own `PeerFrame` enum predates. `Auth`/
/// `AuthOk` exchange this once per session so both sides know whether the
/// other actually supports version-gated frames before sending one; an
/// older peer's `Auth`/`AuthOk` simply omits the field (`#[serde(default)]`
/// -> `0`), which reads as "supports nothing past the original protocol."
pub const PROTOCOL_VERSION: u32 = 3;

/// The fixed protocol version that introduced `PeerFrame::Ping`/`Pong`
/// (ADR-0003 revision) -- deliberately a separate constant from
/// `PROTOCOL_VERSION` above, not an alias for it, for the same reason
/// `BluetoothAddressUpdate`'s `>= 1` gate is: if a later, unrelated feature
/// bumps `PROTOCOL_VERSION` again, a peer on version 3 (which understands
/// Ping/Pong fine, just not whatever feature came after it) must not
/// suddenly fail this check and silently lose the ability to ever reach
/// green.
pub const PING_MIN_PROTOCOL_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PeerFrame {
    #[serde(rename = "auth")]
    Auth {
        device_id: String,
        peer_device_id: String,
        #[serde(default)]
        protocol_version: u32,
    },
    #[serde(rename = "auth_ok")]
    AuthOk {
        #[serde(default)]
        protocol_version: u32,
    },
    #[serde(rename = "auth_fail")]
    AuthFail { reason: String },
    #[serde(rename = "pair_request")]
    PairRequest(PairRequestPayload),
    #[serde(rename = "pair_accept")]
    PairAccept(PairAcceptPayload),
    #[serde(rename = "pair_complete")]
    PairComplete(PairCompletePayload),
    #[serde(rename = "sync_event")]
    SyncEvent(SyncEventEnvelope),
    #[serde(rename = "ack")]
    Ack { event_id: String },
    #[serde(rename = "bootstrap_start")]
    BootstrapStart { space_id: String },
    #[serde(rename = "bootstrap_end")]
    BootstrapEnd {
        space_id: String,
        completed_at: String,
    },
    #[serde(rename = "space_mapping_update")]
    SpaceMappingUpdate {
        mapped_space_ids: Vec<String>,
        custom_spaces: Vec<CustomSpaceDescriptor>,
        sent_at: String,
    },
    #[serde(rename = "space_sync_end")]
    SpaceSyncEnd { space_id: String, ended_at: String },
    /// Sent once by whichever side of an authenticated *network* session can
    /// read its own real Bluetooth adapter address (Linux, via
    /// `bluetoothctl` — Android cannot: `BluetoothAdapter.getAddress()` has
    /// returned a dummy value since Android 6.0 for every normal app, no
    /// workaround exists). Lets the other side learn a usable Bluetooth
    /// fallback address without the user typing it in by hand. See
    /// `docs/adr/0002-bluetooth-address-exchange-live-status-and-ble-pairing.md`.
    #[serde(rename = "bluetooth_address_update")]
    BluetoothAddressUpdate { address: String },
    /// Pre-auth, sent by a scanner over a fresh BLE connection to a
    /// candidate whose advertisement already carried the add-mode flag
    /// (`transport::ble`'s own scan-side filtering, so a stranger not in
    /// add-mode is never even connected to). BLE advertisements can't carry
    /// a device_id/hostname the way mDNS's `DiscoveryBeacon` does (payload
    /// too small alongside the service UUID), and `PairRequestPayload`
    /// itself requires `to_device_id` up front -- this is what lets a
    /// scanner learn it before attempting a real `PairRequest`. Untrusted,
    /// same as `PairRequest`/`PairAccept`/`PairComplete`: discovery
    /// metadata is never the trust boundary, Fini's own pairing handshake
    /// is (`specs/device-connect/README.md`).
    #[serde(rename = "discovery_hello")]
    DiscoveryHello,
    /// Reply to `DiscoveryHello`, sent only if the receiver is currently in
    /// add-mode itself -- `specs/device-connect/README.md`: "Only devices
    /// in add-mode are pairing candidates."
    #[serde(rename = "discovery_hello_reply")]
    DiscoveryHelloReply { device_id: String, hostname: String },
    /// Pre-auth, sent by `transport::ble::find_peer_address` ("Find via
    /// Bluetooth") to confirm a freshly-scanned address genuinely belongs
    /// to an already-paired peer, *without* requiring Bluetooth to already
    /// be enabled for that pair. This can't reuse the ordinary `Auth`/
    /// `AuthOk` handshake: `run_peer_gate`'s Bluetooth branch enforces
    /// `check_bluetooth_enabled` as a precondition, which is exactly the
    /// flag "Find via Bluetooth" exists to help the user turn on in the
    /// first place -- reusing it would mean the discovery flow could never
    /// succeed for its actual target case. Untrusted, same trust model as
    /// `DiscoveryHello`: this only proves "you already know a device_id I
    /// have paired," not cryptographic identity -- Fini's own pairing
    /// handshake remains the trust boundary (`specs/device-connect/README.md`).
    #[serde(rename = "bluetooth_probe")]
    BluetoothProbe { device_id: String },
    /// Reply to `BluetoothProbe`, sent only if the sender's claimed
    /// `device_id` is already one of this device's paired peers
    /// (`check_paired` -- deliberately not `check_bluetooth_enabled`).
    #[serde(rename = "bluetooth_probe_reply")]
    BluetoothProbeReply { device_id: String },
    /// ADR-0003 revision: app-level liveness proof, sent on *every*
    /// connected transport (not just the primary one) every `PING_INTERVAL`
    /// -- see `session::run_session`'s ping loop and
    /// `TransportAckState`'s doc comment for the full green/amber
    /// bookkeeping this drives. Gated behind `PROTOCOL_VERSION >=
    /// PING_MIN_PROTOCOL_VERSION` the same way `BluetoothAddressUpdate` is
    /// gated behind `>= 1` -- an older peer that doesn't understand it
    /// would otherwise decode-fail and drop the whole session (see
    /// `Unknown`'s doc comment); such a peer's transports simply stay
    /// amber forever, which is the correct (if degraded) outcome for a
    /// pre-upgrade peer rather than a dropped connection.
    #[serde(rename = "ping")]
    Ping,
    /// Reply to an inbound `Ping`, sent immediately.
    #[serde(rename = "pong")]
    Pong,
    /// Catches any `type` tag this build doesn't recognize, instead of
    /// failing to decode outright. Without this, a peer running an older
    /// build that unconditionally receives a newer frame kind (e.g.
    /// `BluetoothAddressUpdate`, sent proactively into an already
    /// *authenticated* `run_session` loop) would hit a decode error on
    /// `recv_frame` -- and `run_session`'s `let Some(Ok(frame)) = inbound
    /// else { break }` treats that as fatal, silently ending the whole
    /// sync session rather than just skipping the one frame it didn't
    /// understand. Mixed-version paired devices (one side updated, one
    /// not) are the normal case during a rollout, not an edge case.
    /// `#[serde(other)]` must be a unit variant and is matched only when no
    /// named variant's tag matches.
    #[serde(other)]
    Unknown,
}
