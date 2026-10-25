//! The Tauri commands of `SpaceSyncService`: the spaces a pair syncs, and
//! how far the sync is.

use std::sync::Arc;

use tauri::State;

use crate::models::channel::ChannelKind;
use crate::services::channel_service::CustomSpaceDescriptor;
use crate::services::db::AppDbConnection;
use crate::services::space_sync_service::{
    IncomingSpaceMappingUpdate, SpaceMappingApplyResult, SpaceResolutionMode, SpaceSyncService, SpaceSyncStatus, SpaceSyncTickResult,
    SyncQueueSummary,
};

type SpaceSync<'a> = State<'a, Arc<SpaceSyncService>>;

#[tauri::command]
pub fn space_sync_list_mappings(
    db: State<AppDbConnection>,
    space_sync: SpaceSync,
    peer_device_id: String,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    space_sync.list_mappings(&mut conn, &peer_device_id)
}

#[tauri::command]
pub fn space_sync_update_mappings(
    db: State<AppDbConnection>,
    space_sync: SpaceSync,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    space_sync.update_mappings(&mut conn, &peer_device_id, mapped_space_ids)
}

#[tauri::command]
pub fn space_sync_apply_remote_mappings(
    db: State<AppDbConnection>,
    space_sync: SpaceSync,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<SpaceMappingApplyResult, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    space_sync.apply_remote_mappings(&mut conn, &peer_device_id, mapped_space_ids, custom_spaces)
}

#[tauri::command]
pub fn space_sync_resolve_custom_space_mapping(
    db: State<AppDbConnection>,
    space_sync: SpaceSync,
    peer_device_id: String,
    remote_space_id: String,
    remote_space_name: Option<String>,
    resolution_mode: SpaceResolutionMode,
    existing_space_id: Option<String>,
) -> Result<SpaceMappingApplyResult, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    space_sync.resolve_custom_space_mapping(
        &mut conn,
        &peer_device_id,
        remote_space_id,
        remote_space_name,
        resolution_mode,
        existing_space_id,
    )
}

/// The space mapping requests that arrived, once.
#[tauri::command]
pub fn space_sync_take_mapping_updates(space_sync: SpaceSync) -> Vec<IncomingSpaceMappingUpdate> {
    space_sync.take_space_mapping_updates()
}

/// One sync pass now. A call from the frontend: on Android the Activity is
/// up from here on.
#[tauri::command]
pub async fn space_sync_tick(space_sync: SpaceSync<'_>) -> Result<SpaceSyncTickResult, String> {
    space_sync.record_activity_up();
    let space_sync = space_sync.inner().clone();
    tauri::async_runtime::spawn_blocking(move || space_sync.tick())
        .await
        .map_err(|err| format!("the tick task failed: {err}"))?
}

#[tauri::command]
pub fn space_sync_status(
    db: State<AppDbConnection>,
    space_sync: SpaceSync,
    peer_device_id: Option<String>,
) -> Result<SpaceSyncStatus, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    space_sync.status(&mut conn, peer_device_id)
}

#[tauri::command]
pub fn space_sync_queue_summary(
    db: State<AppDbConnection>,
    space_sync: SpaceSync,
    peer_device_id: String,
) -> Result<SyncQueueSummary, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    space_sync.queue_summary(&mut conn, peer_device_id)
}

/// The channel of a session that runs with this peer now, if any.
#[tauri::command]
pub fn space_sync_session_channel(space_sync: SpaceSync, peer_device_id: String) -> Option<ChannelKind> {
    space_sync.active_session_channel(&peer_device_id)
}
