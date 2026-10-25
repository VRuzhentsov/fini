//! The data of Add Device and of pairing, as the UI reads it.

use serde::{Deserialize, Serialize};

use crate::models::channel::ChannelKind;
use crate::models::peer_frame::CustomSpaceDescriptor;

/// A device that Add Device can list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub hostname: String,
    /// An IP address for Network, a Bluetooth address for Bluetooth.
    pub addr: String,
    pub discovery_port: u16,
    pub ws_port: Option<u16>,
    pub last_seen_at: String,
    /// The channel that heard the device.
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
    /// Whether this `PairStart` arrived over a Bluetooth link. Then
    /// `from_bluetooth_address` is the sender's address as seen on that link.
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
    pub via_bluetooth: bool,
    /// The completing peer's Bluetooth address, if known.
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
    /// The peer's iroh key, for a caller that has not heard the peer on the
    /// network (a one-shot CLI command). TLS proves it either way.
    #[serde(default)]
    pub to_endpoint_id: Option<String>,
}

/// A pair request over Bluetooth: addressed by the Bluetooth address alone.
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

/// One channel's init in progress (ADR-0008 D1). Complete once both halves
/// are true: the peer accepted this device's `AddChannel`, and this device
/// accepted the peer's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSetup {
    /// Which setup this is. A setup closed and opened again for the same
    /// channel is a new attempt, and what the old one's task still brings
    /// back must not count for it.
    #[serde(skip)]
    pub attempt: u64,
    /// The peer accepted this device's `AddChannel`.
    pub sent_request_accepted: bool,
    /// This device accepted the peer's `AddChannel`.
    pub accepted_peer_request: bool,
}

impl ChannelSetup {
    pub fn initialized(&self) -> bool {
        self.sent_request_accepted && self.accepted_peer_request
    }
}
