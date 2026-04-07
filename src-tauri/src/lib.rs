pub mod models;
mod schema;
mod services;
// mod voice;       // postponed
// mod model_download; // postponed

use services::db::{open_db, DbState};
use services::device_connection::{
    device_connection_debug_status, device_connection_discovery_snapshot,
    device_connection_enter_add_mode, device_connection_get_identity,
    device_connection_get_paired_devices, device_connection_leave_add_mode,
    device_connection_pair_accept_request, device_connection_pair_acknowledge_request,
    device_connection_pair_complete_request, device_connection_pair_incoming_requests,
    device_connection_pair_outgoing_completions, device_connection_pair_outgoing_updates,
    device_connection_presence_snapshot, device_connection_save_paired_device,
    device_connection_send_pair_request, device_connection_unpair,
    device_connection_update_last_seen, DeviceConnectionState,
};
use services::quest::{
    create_quest, delete_quest, get_active_focus, get_quests, set_focus, update_quest,
};
use services::reminder::{create_reminder, delete_reminder, get_reminders};
use services::space::{create_space, delete_space, get_spaces, update_space};
use services::space_sync::{
    space_sync_list_mappings, space_sync_status, space_sync_update_mappings,
};
use tauri::Manager;

pub fn run_mcp() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(services::mcp::run())
        .unwrap();
}

pub fn run_cli() -> i32 {
    services::cli::run(std::env::args().collect())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    let mut builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }
    builder
        .setup(|app| {
            let conn = open_db(&app.handle());
            app.manage(DbState(std::sync::Mutex::new(conn)));

            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            app.manage(DeviceConnectionState::new(&data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_spaces,
            create_space,
            update_space,
            delete_space,
            get_quests,
            get_active_focus,
            create_quest,
            set_focus,
            update_quest,
            delete_quest,
            get_reminders,
            create_reminder,
            delete_reminder,
            device_connection_get_identity,
            device_connection_enter_add_mode,
            device_connection_leave_add_mode,
            device_connection_discovery_snapshot,
            device_connection_presence_snapshot,
            device_connection_send_pair_request,
            device_connection_pair_incoming_requests,
            device_connection_pair_outgoing_updates,
            device_connection_pair_outgoing_completions,
            device_connection_pair_accept_request,
            device_connection_pair_complete_request,
            device_connection_pair_acknowledge_request,
            device_connection_debug_status,
            device_connection_get_paired_devices,
            device_connection_save_paired_device,
            device_connection_unpair,
            device_connection_update_last_seen,
            space_sync_list_mappings,
            space_sync_update_mappings,
            space_sync_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
