use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

use crate::services::space_sync::types::{SessionSender, SyncEventEnvelope};
use crate::services::transport::TransportKind;
// Aliased: this module's own `TransportKind` (Network/Bluetooth, above)
// names the same concept `crate::services::transport::TransportKind`
// (TcpWs/Sim/Bluetooth/LoRa, used for `peer_session_kind` below) does at a
// different granularity -- `DiscoveredDevice.transport` only ever needs
// "which discovery mechanism found this," not the live-session kind.
use super::transport::TransportKind as DiscoveryTransportKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub hostname: String,
    pub addr: String,
    pub discovery_port: u16,
    pub ws_port: Option<u16>,
    pub last_seen_at: String,
    /// Which discovery mechanism found this candidate — ADR 0002 Phase 3's
    /// unified candidate list. `discovery_port`/`ws_port` are meaningless
    /// for a Bluetooth-discovered entry (`addr` carries the Bluetooth
    /// address instead of an IP); `#[serde(default)]` on the network side
    /// keeps this additive for any caller still constructing the old
    /// three-field shape.
    #[serde(default)]
    pub transport: DiscoveryTransportKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConnectionDebugStatus {
    pub add_mode_enabled: bool,
    pub worker_started: bool,
    pub tx_count: u64,
    pub rx_count: u64,
    pub discovered_count: usize,
    pub peer_session_count: usize,
    pub incoming_request_count: usize,
    pub incoming_space_mapping_update_count: usize,
    pub outgoing_code_count: usize,
    pub last_broadcast_at: Option<String>,
    pub last_error: Option<String>,
    pub discovery_port: u16,
    pub discovery_provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingPairRequest {
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub created_at: String,
    pub expires_at: String,
    pub attempts: i64,
    pub cooldown_until: Option<String>,
    /// Whether this `PairRequest` arrived over a Bluetooth link (ADR 0002
    /// Phase 3's BLE-first pairing) rather than network. When true,
    /// `from_bluetooth_address` carries the sender's address as *observed*
    /// on this connection (`Link::peer_addr()`), which is more trustworthy
    /// than a self-reported value.
    pub via_bluetooth: bool,
    pub from_bluetooth_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCodeUpdate {
    pub request_id: String,
    pub code: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCompletionUpdate {
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub paired_at: String,
    /// Mirrors `IncomingPairRequest::via_bluetooth` for the completion leg.
    pub via_bluetooth: bool,
    /// The completing peer's Bluetooth address, if known -- either observed
    /// directly (when `via_bluetooth`) or self-reported in the payload
    /// (when completion arrived over network). See
    /// `PairCompletePayload::bluetooth_address`.
    pub bluetooth_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingSpaceMappingUpdate {
    pub from_device_id: String,
    pub mapped_space_ids: Vec<String>,
    pub custom_spaces: Vec<CustomSpaceDescriptor>,
    pub sent_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingSpaceSyncEnd {
    pub from_device_id: String,
    pub space_id: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSpaceDescriptor {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingSyncAck {
    pub from_device_id: String,
    pub event_id: String,
    pub acked_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestInput {
    pub request_id: String,
    pub to_device_id: String,
    pub to_addr: String,
    pub to_ws_port: Option<u16>,
}

/// The BLE-first pairing equivalent of `DevicePairRequestInput` (ADR 0002
/// Phase 3) — no port, since a BLE connection is addressed by MAC alone.
#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestBluetoothInput {
    pub request_id: String,
    pub to_device_id: String,
    pub to_bluetooth_address: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestAckInput {
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceBluetoothTransportInput {
    pub peer_device_id: String,
    pub enabled: bool,
    pub bluetooth_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct DiscoveryBeacon {
    pub protocol: String,
    pub mode: String,
    pub device_id: String,
    pub hostname: String,
    pub sent_at: String,
    #[serde(default)]
    pub discovery_port: Option<u16>,
    #[serde(default)]
    pub ws_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PairRequestPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    #[serde(default)]
    pub from_discovery_port: Option<u16>,
    #[serde(default)]
    pub from_ws_port: Option<u16>,
    pub to_device_id: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PairAcceptPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub code: String,
    pub from_device_id: String,
    pub to_device_id: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PairCompletePayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub to_device_id: String,
    pub paired_at: String,
    /// The completing peer's own local Bluetooth address, if known -- sent
    /// regardless of which transport carries this frame (ADR 0002 Phase 3),
    /// so a network-carried completion can still hand the receiver a
    /// Bluetooth address to store. `#[serde(default)]` keeps this additive
    /// for any peer still running the pre-Phase-3 wire shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluetooth_address: Option<String>,
    /// Reserved for future Signal-style key agreement (X3DH). Unused today;
    /// pass-through `SecureChannel` never populates or reads this. Keeping
    /// the slot on the wire now means enabling encryption later is additive,
    /// not a breaking wire-format change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_material: Option<crate::services::transport::secure_channel::KeyMaterial>,
}

#[derive(Debug, Clone)]
pub(super) struct StoredIncomingPairRequest {
    pub request: IncomingPairRequest,
    pub from_addr: String,
    pub from_ws_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub(super) struct SeenPeer {
    pub hostname: String,
    pub addr: String,
    pub discovery_port: u16,
    pub ws_port: Option<u16>,
    pub last_seen_at: String,
    pub last_seen_mono: Instant,
}

/// ADR-0003 revision: per-(peer, transport) bidirectional ping/ack proof.
/// "Green" (`DeviceConnectionState::transport_reliable`) requires both
/// `own_ping_acked` and `peer_ping_received` true -- this device's own
/// outbound ping was acked (proves this device can send *and* receive), and
/// this device has received a ping from the peer and replied (proves the
/// peer's own send reaches this device). Both sides run the identical
/// protocol at the same 15s cadence (`PING_INTERVAL`), so a healthy link
/// converges both peers to green within one interval of each other without
/// needing an explicit shared/synced value beyond the ping/pong itself.
/// "Continuously re-proven": neither flag is sticky. Each is cleared after
/// 3 consecutive missed cycles on its own side -- reusing Phase 1's
/// interval/threshold shape -- so a link that stops actually exchanging
/// pings decays back to amber within ~45s even if the underlying transport
/// never reports a hard disconnect.
#[derive(Debug, Clone, Default)]
pub(super) struct TransportAckState {
    /// True once a `Pong` has answered this device's own most recent `Ping`.
    pub own_ping_acked: bool,
    /// True while a `Ping` this device sent is still awaiting its `Pong` --
    /// distinguishes "haven't sent one yet" from "sent one, no reply yet"
    /// so the tick handler knows whether a miss occurred.
    pub own_ping_awaiting_pong: bool,
    /// Consecutive tick cycles where an outstanding own-ping went unanswered.
    /// Reset to 0 on any `Pong`; at 3, `own_ping_acked` flips to false.
    pub consecutive_missed_own_pings: u32,
    /// True while an inbound `Ping` from the peer has been seen recently
    /// enough (within `PING_MISS_THRESHOLD` ticks).
    pub peer_ping_received: bool,
    /// Ticks elapsed since the last inbound `Ping` from the peer. Reset to
    /// 0 whenever one arrives; at 3, `peer_ping_received` flips to false.
    pub ticks_since_peer_ping: u32,
}

#[derive(Debug, Default)]
pub(super) struct DiscoveryRuntime {
    pub add_mode_enabled: bool,
    pub worker_started: bool,
    pub tx_count: u64,
    pub rx_count: u64,
    pub last_broadcast_at: Option<String>,
    pub last_error: Option<String>,
    pub presence: HashMap<String, SeenPeer>,
    pub discovered: HashMap<String, SeenPeer>,
    pub incoming_requests: HashMap<String, StoredIncomingPairRequest>,
    pub outgoing_code_updates: HashMap<String, PairCodeUpdate>,
    pub outgoing_pair_completions: HashMap<String, PairCompletionUpdate>,
    pub incoming_space_mapping_updates: HashMap<String, IncomingSpaceMappingUpdate>,
    pub incoming_space_sync_ends: HashMap<String, IncomingSpaceSyncEnd>,
    pub incoming_sync_events: HashMap<String, SyncEventEnvelope>,
    pub incoming_sync_acks: HashMap<String, IncomingSyncAck>,
    /// ADR-0003 revision: one session *per transport*, not one per peer --
    /// both Network and Bluetooth stay connected simultaneously so either
    /// can earn a real (not remembered) green. Keyed by `(peer_device_id,
    /// TransportKind)`. See `DeviceConnectionState::try_claim_session`.
    pub peer_sessions: HashMap<(String, TransportKind), SessionSender>,
    /// Which transport is *primary* for each peer -- the one carrying real
    /// application traffic (SyncEvent, BootstrapEnd, etc.), reported as
    /// `RowState::Live`. Both transports can be connected and green at
    /// once; only one is ever primary. `None` until at least one transport
    /// has claimed a session for this peer.
    pub peer_primary_transport: HashMap<String, TransportKind>,
    /// Per-(peer, transport) bidirectional ping/ack state -- see
    /// `PeerFrame::Ping`/`Pong` and `DeviceConnectionState::transport_ack_state`.
    pub peer_transport_ack: HashMap<(String, TransportKind), TransportAckState>,
    /// Last-known `bluetooth_enabled` per peer, written every time
    /// `recompute_primary_locked` runs (it already reads this from the DB
    /// for its own purposes). A P1 review finding: `reselect_primary_from_
    /// runtime_only` -- the DB-free fallback `release_session` uses so
    /// `peer_primary_transport` never dangles while a fallible DB read is
    /// still in flight -- otherwise has no way to know Bluetooth was *just*
    /// disabled (its session may still be in `peer_sessions`, since
    /// `close_session_on`'s `Close` is delivered asynchronously) and could
    /// pick it as the fallback primary anyway. Missing entry is treated as
    /// `false` (fail closed, matching the disable contract's own
    /// direction) -- self-corrects within one DB-backed recompute either
    /// way, so a brief false negative here is far cheaper than a false
    /// positive routing real traffic over an opted-out transport.
    pub peer_bluetooth_enabled_cache: HashMap<String, bool>,
}
