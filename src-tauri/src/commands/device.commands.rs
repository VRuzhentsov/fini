//! The Tauri commands of `DeviceService`: Add Device, pairing, the paired
//! devices, and the channels of a pair.

use std::sync::Arc;

use tauri::State;

use crate::models::channel::{ChannelKind, ChannelStatus};
use crate::models::pairing::{
    ChannelSetup, DeviceConnectionDebugStatus, DevicePairRequestAckInput, DevicePairRequestBluetoothInput,
    DevicePairRequestInput, DiscoveredDevice, IncomingPairRequest, PairCodeUpdate, PairCompletionUpdate,
};
use crate::models::{Device, Identity};
use crate::services::db::AppDbConnection;
use crate::services::device_service::DeviceService;
use crate::services::space_sync_service::SpaceSyncService;

type Devices<'a> = State<'a, Arc<DeviceService>>;
type SpaceSync<'a> = State<'a, Arc<SpaceSyncService>>;

/// Runs a blocking call of the device service off the main thread: a
/// Bluetooth connect takes seconds, and a blocked command freezes the
/// webview.
async fn off_main_thread<T: Send + 'static>(
    devices: &Arc<DeviceService>,
    call: impl FnOnce(&DeviceService) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let devices = devices.clone();
    tauri::async_runtime::spawn_blocking(move || call(&devices))
        .await
        .map_err(|err| format!("the task failed: {err}"))?
}

fn statuses(
    db: &AppDbConnection,
    devices: &DeviceService,
    space_sync: &SpaceSyncService,
    peer_device_id: &str,
) -> Result<Vec<ChannelStatus>, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    devices.channel_statuses(&mut conn, peer_device_id, |kind| space_sync.has_session_on(peer_device_id, kind))
}

#[tauri::command]
pub fn device_get_identity(devices: Devices) -> Identity {
    devices.identity().clone()
}

#[tauri::command]
pub fn device_enter_add_mode(devices: Devices, bluetooth: bool) -> Result<(), String> {
    devices.set_add_device_mode(true, bluetooth)
}

#[tauri::command]
pub fn device_leave_add_mode(devices: Devices) -> Result<(), String> {
    devices.set_add_device_mode(false, true)
}

#[tauri::command]
pub fn device_available_devices(devices: Devices) -> Vec<DiscoveredDevice> {
    devices.available_devices()
}

#[tauri::command]
pub async fn device_identify_bluetooth_device(devices: Devices<'_>, device_id: String) -> Result<DiscoveredDevice, String> {
    devices.identify_bluetooth_device(&device_id).await
}

#[tauri::command]
pub async fn device_send_pair_request(devices: Devices<'_>, input: DevicePairRequestInput) -> Result<(), String> {
    off_main_thread(&devices, move |devices| devices.send_pair_request(input)).await
}

#[tauri::command]
pub async fn device_send_pair_request_bluetooth(
    devices: Devices<'_>,
    input: DevicePairRequestBluetoothInput,
) -> Result<(), String> {
    off_main_thread(&devices, move |devices| devices.send_pair_request_bluetooth(input)).await
}

#[tauri::command]
pub fn device_pair_incoming_requests(devices: Devices) -> Result<Vec<IncomingPairRequest>, String> {
    devices.incoming_requests()
}

#[tauri::command]
pub fn device_pair_outgoing_updates(devices: Devices) -> Result<Vec<PairCodeUpdate>, String> {
    devices.outgoing_updates()
}

#[tauri::command]
pub fn device_pair_outgoing_completions(devices: Devices) -> Result<Vec<PairCompletionUpdate>, String> {
    devices.outgoing_completions()
}

#[tauri::command]
pub async fn device_pair_accept_request(devices: Devices<'_>, input: DevicePairRequestAckInput) -> Result<PairCodeUpdate, String> {
    off_main_thread(&devices, move |devices| devices.accept_pair_request(input)).await
}

#[tauri::command]
pub async fn device_pair_complete_request(devices: Devices<'_>, input: DevicePairRequestAckInput) -> Result<(), String> {
    off_main_thread(&devices, move |devices| devices.complete_pair_request(input)).await
}

#[tauri::command]
pub fn device_pair_acknowledge_request(devices: Devices, input: DevicePairRequestAckInput) -> Result<(), String> {
    devices.acknowledge_pair_request(input)
}

#[tauri::command]
pub fn device_debug_status(devices: Devices, space_sync: SpaceSync) -> Result<DeviceConnectionDebugStatus, String> {
    let mut status = devices.debug_status()?;
    status.peer_session_count = space_sync.session_count();
    status.incoming_space_mapping_update_count = space_sync.space_mapping_update_count();
    status.discovered_count = devices.available_devices().len();
    Ok(status)
}

#[tauri::command]
pub fn device_paired_devices(devices: Devices) -> Result<Vec<Device>, String> {
    devices.paired_devices()
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn device_save_paired_device(
    db: State<AppDbConnection>,
    devices: Devices,
    peer_device_id: String,
    display_name: String,
    bluetooth_address: Option<String>,
    via_bluetooth: bool,
    request_id: Option<String>,
    endpoint_id: Option<String>,
) -> Result<Device, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    devices.save_paired_device(
        &mut conn,
        &peer_device_id,
        &display_name,
        bluetooth_address,
        via_bluetooth,
        request_id,
        endpoint_id,
    )
}

#[tauri::command]
pub fn device_unpair(db: State<AppDbConnection>, devices: Devices, peer_device_id: String) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    devices.unpair(&mut conn, &peer_device_id)
}

#[tauri::command]
pub fn device_update_last_seen(
    db: State<AppDbConnection>,
    devices: Devices,
    peer_device_id: String,
    last_seen_at: String,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    devices.update_last_seen(&mut conn, &peer_device_id, &last_seen_at)
}

#[tauri::command]
pub fn device_channel_statuses(
    db: State<AppDbConnection>,
    devices: Devices,
    space_sync: SpaceSync,
    peer_device_id: String,
) -> Result<Vec<ChannelStatus>, String> {
    statuses(&db, &devices, &space_sync, &peer_device_id)
}

#[tauri::command]
pub async fn device_set_channel_enabled(
    db: State<'_, AppDbConnection>,
    devices: Devices<'_>,
    space_sync: SpaceSync<'_>,
    peer_device_id: String,
    kind: ChannelKind,
    enabled: bool,
) -> Result<Vec<ChannelStatus>, String> {
    devices.set_channel_enabled(&peer_device_id, kind, enabled).await?;
    statuses(&db, &devices, &space_sync, &peer_device_id)
}

#[tauri::command]
pub fn device_unlink_channel(
    db: State<AppDbConnection>,
    devices: Devices,
    space_sync: SpaceSync,
    peer_device_id: String,
    kind: ChannelKind,
) -> Result<Vec<ChannelStatus>, String> {
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        devices.unlink_channel(&mut conn, &peer_device_id, kind)?;
    }
    statuses(&db, &devices, &space_sync, &peer_device_id)
}

#[tauri::command]
pub fn device_set_primary_channel(
    db: State<AppDbConnection>,
    devices: Devices,
    space_sync: SpaceSync,
    peer_device_id: String,
    primary: Option<ChannelKind>,
) -> Result<Vec<ChannelStatus>, String> {
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        devices.set_primary_channel(&mut conn, &peer_device_id, primary)?;
    }
    statuses(&db, &devices, &space_sync, &peer_device_id)
}

/// Where a channel's init stands on this device (ADR-0008 D1).
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSetupStatus {
    pub sent_request_accepted: bool,
    pub accepted_peer_request: bool,
    /// Both halves done: OK can be pressed (D4).
    pub initialized: bool,
}

impl From<ChannelSetup> for ChannelSetupStatus {
    fn from(setup: ChannelSetup) -> Self {
        Self {
            sent_request_accepted: setup.sent_request_accepted,
            accepted_peer_request: setup.accepted_peer_request,
            initialized: setup.initialized(),
        }
    }
}

#[tauri::command]
pub async fn device_begin_channel_setup(devices: Devices<'_>, peer_device_id: String, kind: ChannelKind) -> Result<(), String> {
    devices.begin_channel_setup(&peer_device_id, kind).await
}

#[tauri::command]
pub fn device_channel_setup_status(devices: Devices, peer_device_id: String, kind: ChannelKind) -> Option<ChannelSetupStatus> {
    devices.channel_setup(&peer_device_id, kind).map(ChannelSetupStatus::from)
}

#[tauri::command]
pub fn device_end_channel_setup(
    db: State<AppDbConnection>,
    devices: Devices,
    space_sync: SpaceSync,
    peer_device_id: String,
    kind: ChannelKind,
    switch_on: bool,
) -> Result<Vec<ChannelStatus>, String> {
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        devices.end_channel_setup(&mut conn, &peer_device_id, kind, switch_on)?;
    }
    statuses(&db, &devices, &space_sync, &peer_device_id)
}
