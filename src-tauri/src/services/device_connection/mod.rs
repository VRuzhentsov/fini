mod commands;
mod runtime;
mod types;

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub use commands::{
    device_connection_debug_status, device_connection_discovery_snapshot,
    device_connection_enter_add_mode, device_connection_get_identity,
    device_connection_get_paired_devices, device_connection_leave_add_mode,
    device_connection_pair_accept_request, device_connection_pair_acknowledge_request,
    device_connection_pair_complete_request, device_connection_pair_incoming_requests,
    device_connection_pair_outgoing_completions, device_connection_pair_outgoing_updates,
    device_connection_presence_snapshot, device_connection_save_paired_device,
    device_connection_send_pair_request, device_connection_unpair,
    device_connection_update_last_seen,
};
use runtime::{load_or_create_identity, spawn_discovery_worker};
pub use types::DeviceIdentity;
use types::DiscoveryRuntime;

pub const DISCOVERY_INTERVAL_MS: u64 = 5_000;
pub const HEARTBEAT_INTERVAL_MS: u64 = 60_000;

pub(super) const DISCOVERY_PROTOCOL: &str = "fini-device-sync-v1";
pub(super) const DISCOVERY_PORT: u16 = 45_454;
pub(super) const DISCOVERY_TTL_SECS: u64 = 15;
pub(super) const PAIR_REQUEST_TTL_SECS: i64 = 60;
pub(super) const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
pub(super) const SPACE_SYNC_WS_PORT: u16 = 45_455;

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
}
