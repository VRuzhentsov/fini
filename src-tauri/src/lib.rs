pub mod models;
mod schema;
mod services;
// mod voice;       // postponed
// mod model_download; // postponed

use services::db::{app_data_dir, open_db, DbState};
use services::device_connection::{
    device_connection_consume_space_mapping_updates, device_connection_debug_status,
    device_connection_discovery_snapshot, device_connection_enter_add_mode,
    device_connection_get_identity, device_connection_get_paired_devices,
    device_connection_leave_add_mode, device_connection_pair_accept_request,
    device_connection_pair_acknowledge_request, device_connection_pair_complete_request,
    device_connection_pair_incoming_requests, device_connection_pair_outgoing_completions,
    device_connection_pair_outgoing_updates, device_connection_presence_snapshot,
    device_connection_save_paired_device, device_connection_send_pair_request,
    device_connection_unpair, device_connection_update_last_seen, DeviceConnectionState,
};
#[cfg(feature = "e2e-testing")]
use services::notification::{
    e2e_clear_notification_events, e2e_list_notification_events, NotificationObserverState,
};
use services::notification::{setup_notification_channel, SchedulerState};
use services::quest::{
    create_quest, delete_quest, get_active_focus, get_quests, set_focus, update_quest,
};
use services::reconciler;
use services::reminder::{
    cancel_quest_notifications, create_reminder, delete_reminder, get_reminders, update_reminder,
};
use services::space::{create_space, delete_space, get_spaces, update_space};
use services::space_sync::{
    run_ws_server, space_sync_apply_remote_mappings, space_sync_list_mappings,
    space_sync_resolve_custom_space_mapping, space_sync_status, space_sync_tick,
    space_sync_update_mappings,
};
use tauri::Manager;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tauri_plugin_autostart::ManagerExt;

#[cfg(target_os = "linux")]
fn linux_prefers_dark() -> bool {
    std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            "color-scheme",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|stdout| stdout.contains("uint32 1"))
}

#[tauri::command]
fn theme_hint() -> String {
    if std::env::var_os("FLATPAK_ID").is_some() {
        return "dark".to_string();
    }

    #[cfg(target_os = "linux")]
    if linux_prefers_dark() {
        return "dark".to_string();
    }

    "system".to_string()
}

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

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ));
    #[cfg(debug_assertions)]
    let builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    #[cfg(feature = "e2e-testing")]
    let builder = {
        let socket_path = std::env::var("TAURI_PLAYWRIGHT_SOCKET")
            .unwrap_or_else(|_| "/tmp/tauri-playwright.sock".to_string());
        builder.plugin(tauri_plugin_playwright::init_with_config(
            tauri_plugin_playwright::PluginConfig::new().socket_path(&socket_path),
        ))
    };
    builder
        .setup(|app| {
            let app_handle = app.handle();

            let conn = open_db(&app_handle);
            app.manage(DbState(std::sync::Mutex::new(conn)));
            app.manage(SchedulerState::new());
            #[cfg(feature = "e2e-testing")]
            app.manage(NotificationObserverState::new());

            setup_notification_channel(&app_handle);

            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
            if let Err(e) = app_handle.autolaunch().enable() {
                eprintln!("[autostart] enable failed: {e}");
            }

            let db_state = app.state::<DbState>();
            reconciler::run(&app_handle, &db_state);

            let data_dir = app_data_dir(&app_handle);
            let dc_state = DeviceConnectionState::new(&data_dir);
            tauri::async_runtime::spawn(run_ws_server(dc_state.clone(), dc_state.db_path.clone()));
            app.manage(dc_state);
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
            update_reminder,
            delete_reminder,
            cancel_quest_notifications,
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
            device_connection_consume_space_mapping_updates,
            space_sync_list_mappings,
            space_sync_update_mappings,
            space_sync_apply_remote_mappings,
            space_sync_resolve_custom_space_mapping,
            space_sync_tick,
            space_sync_status,
            theme_hint,
            #[cfg(feature = "e2e-testing")]
            e2e_list_notification_events,
            #[cfg(feature = "e2e-testing")]
            e2e_clear_notification_events,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
