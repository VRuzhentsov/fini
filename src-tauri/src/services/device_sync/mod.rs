mod commands;
mod runtime;
mod types;

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub use commands::{
    device_discovery_snapshot, device_enter_add_mode, device_get_identity, device_leave_add_mode,
    device_pair_accept_request, device_pair_acknowledge_request, device_pair_complete_request,
    device_pair_incoming_requests, device_pair_outgoing_completions, device_pair_outgoing_updates,
    device_presence_snapshot, device_send_pair_request, device_sync_debug_status,
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

pub struct DeviceSyncState {
    pub identity: DeviceIdentity,
    runtime: Arc<Mutex<DiscoveryRuntime>>,
}

impl DeviceSyncState {
    pub fn new(app_data_dir: &Path) -> Self {
        let identity = load_or_create_identity(app_data_dir);
        let runtime = Arc::new(Mutex::new(DiscoveryRuntime::default()));

        spawn_discovery_worker(identity.clone(), runtime.clone());

        Self { identity, runtime }
    }
}
