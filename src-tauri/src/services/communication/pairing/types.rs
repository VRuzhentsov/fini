use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

use crate::services::communication::sync::types::{PeerFrame, SessionSender, SyncEventEnvelope};
// One kind, at one granularity: the channel a person chose is the same
// thing the live session runs on. This used to be two types -- an adapter
// kind and a row kind -- which is why several comments nearby drew a
// distinction that no longer exists.
use crate::services::communication::channel::ChannelKind;

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
    /// Which channel found this candidate — ADR 0002 Phase 3's unified
    /// candidate list. `discovery_port`/`ws_port` are meaningless for a
    /// Bluetooth-discovered entry (`addr` carries the Bluetooth address
    /// instead of an IP); `#[serde(default)]` on the network side keeps
    /// this additive for any caller still constructing the old three-field
    /// shape.
    #[serde(default)]
    pub channel_kind: ChannelKind,
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
    /// on this connection (`DataLink::peer_addr()`), which is more trustworthy
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
#[cfg(any(feature = "ui-plane", test))]
pub struct DevicePairRequestBluetoothInput {
    pub request_id: String,
    pub to_device_id: String,
    pub to_bluetooth_address: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestAckInput {
    pub request_id: String,
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
    /// regardless of which channel carries this frame (ADR 0002 Phase 3),
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
    pub key_material: Option<crate::services::communication::channel::encryption::KeyMaterial>,
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
    /// The exchanges running right now, one per (peer, channel) at most
    /// (ADR-0008 D10). Each ends on its own once idle.
    pub peer_sessions: HashMap<(String, ChannelKind), SessionSender>,
    /// Frames waiting for the next exchange with each peer -- a mapping
    /// update or a space-sync end raised while nothing was connected.
    pub pending_frames: HashMap<String, Vec<PeerFrame>>,
    /// Whether the last presence beacon failed to go out at all: the
    /// Network channel's problem signal (ADR-0008 D19).
    pub network_broadcast_failing: bool,
    /// ADR-0008 D1/D2: the channels this device is running a setup search
    /// for right now, and how far each one's init has got.
    pub channel_setups: HashMap<(String, ChannelKind), ChannelSetup>,
}

/// One channel's init in progress (ADR-0008 D1). Complete once both halves
/// are true: this device's hello was acknowledged by the peer, and this
/// device acknowledged the peer's hello.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSetup {
    /// Which setup this is. A search closed and reopened for the same
    /// channel is a new attempt, and what the old one's task still brings
    /// back must not count for it.
    #[serde(skip)]
    pub attempt: u64,
    /// The peer answered this device's hello.
    pub hello_acked_by_peer: bool,
    /// This device answered the peer's hello.
    pub acked_peer_hello: bool,
}

impl ChannelSetup {
    #[cfg(any(feature = "ui-plane", test))]
    pub fn initialized(&self) -> bool {
        self.hello_acked_by_peer && self.acked_peer_hello
    }
}
