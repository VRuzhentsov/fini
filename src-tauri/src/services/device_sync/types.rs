use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

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
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSyncDebugStatus {
    pub add_mode_enabled: bool,
    pub worker_started: bool,
    pub tx_count: u64,
    pub rx_count: u64,
    pub discovered_count: usize,
    pub incoming_request_count: usize,
    pub outgoing_code_count: usize,
    pub last_broadcast_at: Option<String>,
    pub last_error: Option<String>,
    pub discovery_port: u16,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePairRequestInput {
    pub request_id: String,
    pub to_device_id: String,
    pub to_addr: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PairRequestPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub to_device_id: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PairAcceptPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub code: String,
    pub from_device_id: String,
    pub to_device_id: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PairCompletePayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub to_device_id: String,
    pub paired_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct StoredIncomingPairRequest {
    pub request: IncomingPairRequest,
    pub from_addr: String,
}

#[derive(Debug, Clone)]
pub(super) struct SeenPeer {
    pub hostname: String,
    pub addr: String,
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
}
