pub mod models;
mod schema;
mod services;
// mod voice;       // postponed
// mod model_download; // postponed

use services::db::{open_db, DbState};
use services::device_sync::{
    device_discovery_snapshot, device_enter_add_mode, device_get_identity, device_leave_add_mode,
    device_pair_accept_request, device_pair_acknowledge_request, device_pair_complete_request,
    device_pair_incoming_requests, device_pair_outgoing_completions, device_pair_outgoing_updates,
    device_presence_snapshot, device_send_pair_request, device_sync_debug_status, DeviceSyncState,
};
use services::quest::{
    create_quest, delete_quest, get_active_quest, get_quests, set_main_quest, update_quest,
};
use services::reminder::{create_reminder, delete_reminder, get_reminders};
use services::space::{create_space, delete_space, get_spaces, update_space};
use tauri::Manager;

pub fn run_mcp() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(services::mcp::run())
        .unwrap();
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
            app.manage(DeviceSyncState::new(&data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_spaces,
            create_space,
            update_space,
            delete_space,
            get_quests,
            get_active_quest,
            create_quest,
            set_main_quest,
            update_quest,
            delete_quest,
            get_reminders,
            create_reminder,
            delete_reminder,
            device_get_identity,
            device_enter_add_mode,
            device_leave_add_mode,
            device_discovery_snapshot,
            device_presence_snapshot,
            device_send_pair_request,
            device_pair_incoming_requests,
            device_pair_outgoing_updates,
            device_pair_outgoing_completions,
            device_pair_accept_request,
            device_pair_complete_request,
            device_pair_acknowledge_request,
            device_sync_debug_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
