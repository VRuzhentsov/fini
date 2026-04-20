mod commands;
mod runtime;
mod types;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::services::space_sync::types::{SessionSender, SyncEventEnvelope, WsMessage};

pub use commands::{
    device_connection_consume_space_mapping_updates, device_connection_debug_status,
    device_connection_discovery_snapshot, device_connection_enter_add_mode,
    device_connection_get_identity, device_connection_get_paired_devices,
    device_connection_leave_add_mode, device_connection_pair_accept_request,
    device_connection_pair_acknowledge_request, device_connection_pair_complete_request,
    device_connection_pair_incoming_requests, device_connection_pair_outgoing_completions,
    device_connection_pair_outgoing_updates, device_connection_presence_snapshot,
    device_connection_save_paired_device, device_connection_send_pair_request,
    device_connection_unpair, device_connection_update_last_seen,
};
use runtime::{load_or_create_identity, spawn_discovery_worker};
use types::DiscoveryRuntime;
pub use types::{CustomSpaceDescriptor, DeviceIdentity, IncomingSpaceMappingUpdate, IncomingSyncAck};

pub const DISCOVERY_INTERVAL_MS: u64 = 5_000;
pub const HEARTBEAT_INTERVAL_MS: u64 = 60_000;

pub(crate) const DISCOVERY_PROTOCOL: &str = "fini-device-sync-v1";
pub(crate) const DISCOVERY_PORT: u16 = 45_454;
pub(super) const DISCOVERY_TTL_SECS: u64 = 15;
pub(super) const PAIR_REQUEST_TTL_SECS: i64 = 60;
pub(super) const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
pub(crate) const SPACE_SYNC_WS_PORT: u16 = 45_455;

#[derive(Clone)]
pub struct DeviceConnectionState {
    pub identity: DeviceIdentity,
    pub db_path: PathBuf,
    runtime: Arc<Mutex<DiscoveryRuntime>>,
}

impl DeviceConnectionState {
    pub fn new(app_data_dir: &Path) -> Self {
        let identity = load_or_create_identity(app_data_dir);
        let runtime = Arc::new(Mutex::new(DiscoveryRuntime::default()));
        let db_path = app_data_dir.join("fini.db");

        spawn_discovery_worker(identity.clone(), runtime.clone());

        Self {
            identity,
            db_path,
            runtime,
        }
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

    pub fn push_incoming_sync_event(&self, envelope: SyncEventEnvelope) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard
                .incoming_sync_events
                .insert(envelope.event_id.clone(), envelope);
        }
    }

    pub fn push_incoming_sync_ack(&self, ack: IncomingSyncAck) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard
                .incoming_sync_acks
                .insert(ack.event_id.clone(), ack);
        }
    }

    pub fn push_incoming_space_mapping_update(&self, update: IncomingSpaceMappingUpdate) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard
                .incoming_space_mapping_updates
                .insert(update.from_device_id.clone(), update);
        }
    }

    pub fn register_session(&self, peer_device_id: String, sender: SessionSender) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.peer_sessions.insert(peer_device_id, sender);
        }
    }

    pub fn unregister_session(&self, peer_device_id: &str) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.peer_sessions.remove(peer_device_id);
        }
    }

    pub fn push_to_peer(&self, peer_device_id: &str, msg: WsMessage) -> bool {
        let guard = match self.runtime.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if let Some(sender) = guard.peer_sessions.get(peer_device_id) {
            sender.try_send(msg).is_ok()
        } else {
            false
        }
    }

    pub fn has_session(&self, peer_device_id: &str) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard.peer_sessions.contains_key(peer_device_id)
    }

    /// Returns (device_id, addr) for every presenced peer (seen within TTL).
    pub fn list_presenced_peers(&self) -> Vec<(String, String)> {
        let Ok(guard) = self.runtime.lock() else {
            return Vec::new();
        };
        guard
            .presence
            .iter()
            .map(|(id, peer)| (id.clone(), peer.addr.clone()))
            .collect()
    }
}
