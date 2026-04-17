mod commands;
mod runtime;
mod types;

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::services::space_sync::types::SyncEventEnvelope;

pub use commands::{
    device_connection_consume_space_mapping_updates,
    device_connection_consume_space_mapping_updates_impl, device_connection_debug_status,
    device_connection_debug_status_impl, device_connection_discovery_snapshot,
    device_connection_discovery_snapshot_impl, device_connection_enter_add_mode,
    device_connection_enter_add_mode_impl, device_connection_get_identity,
    device_connection_get_identity_impl, device_connection_get_paired_devices,
    device_connection_get_paired_devices_impl, device_connection_leave_add_mode,
    device_connection_leave_add_mode_impl, device_connection_pair_accept_request,
    device_connection_pair_accept_request_impl, device_connection_pair_acknowledge_request,
    device_connection_pair_acknowledge_request_impl, device_connection_pair_complete_request,
    device_connection_pair_complete_request_impl, device_connection_pair_incoming_requests,
    device_connection_pair_incoming_requests_impl, device_connection_pair_outgoing_completions,
    device_connection_pair_outgoing_completions_impl, device_connection_pair_outgoing_updates,
    device_connection_pair_outgoing_updates_impl, device_connection_presence_snapshot,
    device_connection_presence_snapshot_impl, device_connection_save_paired_device,
    device_connection_save_paired_device_impl, device_connection_send_pair_request,
    device_connection_send_pair_request_impl, device_connection_unpair,
    device_connection_unpair_impl, device_connection_update_last_seen,
};
use runtime::{load_or_create_identity, spawn_discovery_worker};
use types::DiscoveryRuntime;
pub use types::{
    DeviceIdentity, DevicePairRequestAckInput, DevicePairRequestInput, IncomingSyncAck,
};

pub const DISCOVERY_INTERVAL_MS: u64 = 5_000;
pub const HEARTBEAT_INTERVAL_MS: u64 = 60_000;

pub(crate) const DISCOVERY_PROTOCOL: &str = "fini-device-sync-v1";
pub(crate) const DISCOVERY_PORT: u16 = 45_454;
pub(super) const DISCOVERY_TTL_SECS: u64 = 15;
pub(super) const PAIR_REQUEST_TTL_SECS: i64 = 60;
pub(super) const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
pub(crate) const SPACE_SYNC_WS_PORT: u16 = 45_455;
pub(crate) const SPACE_MAPPING_UPDATE_KIND: &str = "space_mapping_update";
pub(crate) const SYNC_EVENT_KIND: &str = "sync_event";
pub(crate) const SYNC_ACK_KIND: &str = "sync_ack";

pub struct DeviceConnectionState {
    pub identity: DeviceIdentity,
    runtime: Arc<Mutex<DiscoveryRuntime>>,
}

impl DeviceConnectionState {
    pub fn new(app_data_dir: &Path) -> Self {
        let identity = load_or_create_identity(app_data_dir);
        let runtime = Arc::new(Mutex::new(DiscoveryRuntime::default()));

        spawn_discovery_worker(identity.clone(), runtime.clone());

        Self { identity, runtime }
    }

    pub fn resolve_peer_addr(&self, peer_device_id: &str) -> Option<String> {
        let guard = self.runtime.lock().ok()?;
        guard
            .presence
            .get(peer_device_id)
            .or_else(|| guard.discovered.get(peer_device_id))
            .map(|peer| peer.addr.clone())
    }

    pub fn take_incoming_sync_events(&self) -> Vec<SyncEventEnvelope> {
        let Ok(mut guard) = self.runtime.lock() else {
            return Vec::new();
        };
        let mut events: Vec<SyncEventEnvelope> =
            guard.incoming_sync_events.drain().map(|(_, v)| v).collect();
        events.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        events
    }

    pub fn restore_incoming_sync_events(&self, events: Vec<SyncEventEnvelope>) {
        let Ok(mut guard) = self.runtime.lock() else {
            return;
        };

        for event in events {
            guard
                .incoming_sync_events
                .insert(event.event_id.clone(), event);
        }
    }

    pub fn take_incoming_sync_acks(&self) -> Vec<IncomingSyncAck> {
        let Ok(mut guard) = self.runtime.lock() else {
            return Vec::new();
        };
        let mut acks: Vec<IncomingSyncAck> =
            guard.incoming_sync_acks.drain().map(|(_, v)| v).collect();
        acks.sort_by(|a, b| {
            a.acked_at
                .cmp(&b.acked_at)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        acks
    }
}
