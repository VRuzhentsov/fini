pub mod models;
mod schema;
mod services;
mod utils;
// mod voice;       // postponed
// mod model_download; // postponed

#[cfg(feature = "ui-plane")]
use services::backup::{backup_apply_import, backup_export, backup_preflight_import};
#[cfg(feature = "ui-plane")]
use services::db::{app_data_dir, try_open_db, AppDbConnection};
#[cfg(feature = "ui-plane")]
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
#[cfg(feature = "ui-plane")]
use services::notification::{
    dispatch_action, setup_notifications, SchedulerState, FOREGROUND_FIRE_EVENT, TAP_EVENT,
};
#[cfg(all(feature = "ui-plane", feature = "devtools"))]
use services::notification::{
    e2e_clear_notification_events, e2e_dispatch_notification_action, e2e_list_notification_events,
    NotificationObserverState,
};
#[cfg(feature = "ui-plane")]
mod resume_watcher;
#[cfg(feature = "ui-plane")]
use services::quest::{
    create_quest, delete_quest, delete_quest_series, get_active_focus, get_quests, set_focus,
    update_quest,
};
#[cfg(feature = "ui-plane")]
use services::reconciler;
#[cfg(feature = "ui-plane")]
use services::reminder::{
    cancel_quest_notifications, create_reminder, delete_reminder, get_reminders, update_reminder,
};
#[cfg(feature = "ui-plane")]
use services::settings::{self, ThemeMode};
#[cfg(feature = "ui-plane")]
use services::space::{create_space, delete_space, get_spaces, update_space};
#[cfg(feature = "ui-plane")]
use services::space_sync::{
    run_ws_server, space_sync_apply_remote_mappings, space_sync_list_mappings,
    space_sync_resolve_custom_space_mapping, space_sync_status, space_sync_tick,
    space_sync_update_mappings,
};
#[cfg(feature = "ui-plane")]
use tauri::{AppHandle, Emitter, Manager};
#[cfg(all(
    feature = "ui-plane",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
use tauri_plugin_autostart::ManagerExt;

#[cfg(feature = "ui-plane")]
const THEME_EVENT: &str = "theme://changed";

#[cfg(feature = "ui-plane")]
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupRecovery {
    kind: &'static str,
    title: &'static str,
    message: String,
}

#[cfg(feature = "ui-plane")]
struct StartupRecoveryState(std::sync::Mutex<Option<StartupRecovery>>);

#[cfg(feature = "ui-plane")]
fn unsupported_schema_startup_recovery(error: String) -> Option<StartupRecovery> {
    if !error.contains("database schema is not supported by this Fini binary") {
        return None;
    }

    Some(StartupRecovery {
        kind: "update-required",
        title: "Update required",
        message: error,
    })
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn startup_recovery(
    state: tauri::State<StartupRecoveryState>,
) -> Result<Option<StartupRecovery>, String> {
    state
        .0
        .lock()
        .map(|recovery| recovery.clone())
        .map_err(|err| format!("failed to read startup recovery state: {err}"))
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn notification_action(app: AppHandle, action_id: String, reminder_id: String) {
    dispatch_action(&app, &action_id, &reminder_id);
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn notification_tap(app: AppHandle, reminder_id: String) {
    dispatch_action(&app, "tap", &reminder_id);
}

// Suppress unused-constant warnings; these are used by frontend listeners.
#[cfg(feature = "ui-plane")]
const _: &str = FOREGROUND_FIRE_EVENT;
#[cfg(feature = "ui-plane")]
const _: &str = TAP_EVENT;

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn get_theme_mode(db: tauri::State<AppDbConnection>) -> Result<String, String> {
    let mut conn = db.0.lock().unwrap();
    settings::theme_mode(&mut conn).map(|mode| mode.as_str().to_string())
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn set_theme_mode(
    app: AppHandle,
    db: tauri::State<AppDbConnection>,
    mode: String,
) -> Result<String, String> {
    let mode = ThemeMode::parse(&mode).ok_or_else(|| "invalid theme mode".to_string())?;
    let mut conn = db.0.lock().unwrap();
    let mode = settings::set_theme_mode(&mut conn, mode)?;
    let effective = settings::theme_hint(&mut conn);
    settings::apply_native_theme(&app, &effective);
    let _ = app.emit(THEME_EVENT, effective.clone());
    Ok(mode.as_str().to_string())
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn theme_hint(db: tauri::State<AppDbConnection>) -> String {
    let mut conn = db.0.lock().unwrap();
    settings::theme_hint(&mut conn)
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn get_auto_update_enabled(db: tauri::State<AppDbConnection>) -> Result<bool, String> {
    let mut conn = db.0.lock().unwrap();
    settings::automatic_updates_enabled(&mut conn)
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn set_auto_update_enabled(
    db: tauri::State<AppDbConnection>,
    enabled: bool,
) -> Result<bool, String> {
    let mut conn = db.0.lock().unwrap();
    settings::set_automatic_updates_enabled(&mut conn, enabled)
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn startup_auto_update_supported() -> bool {
    services::desktop_update::startup_auto_update_supported()
}

#[cfg(feature = "ui-plane")]
#[tauri::command]
fn sync_native_theme(app: AppHandle, theme: String) {
    settings::apply_native_theme(&app, &theme);
}

#[cfg(feature = "cli-plane")]
pub fn run_cli() -> i32 {
    services::cli::run(std::env::args().collect())
}

#[cfg(feature = "ui-plane")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init());
    #[cfg(all(
        feature = "desktop-updater",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(all(
        any(target_os = "linux", target_os = "macos", target_os = "windows"),
        not(debug_assertions)
    ))]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ));
    #[cfg(feature = "devtools")]
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

            match try_open_db(&app_handle) {
                Ok(conn) => {
                    app.manage(StartupRecoveryState(std::sync::Mutex::new(None)));
                    app.manage(AppDbConnection(std::sync::Mutex::new(conn)));
                    let auto_updates_enabled = {
                        let db = app.state::<AppDbConnection>();
                        let mut conn = db.0.lock().unwrap();
                        settings::automatic_updates_enabled(&mut conn).unwrap_or(true)
                    };
                    services::desktop_update::spawn_startup_auto_update(
                        &app_handle,
                        auto_updates_enabled,
                    );
                    #[cfg(target_os = "linux")]
                    if let Err(error) =
                        services::appimage_desktop::self_register_appimage_desktop_entry()
                    {
                        eprintln!("[appimage-desktop] self-registration failed: {error}");
                    }
                }
                Err(error) => match unsupported_schema_startup_recovery(error.clone()) {
                    Some(recovery) => {
                        services::desktop_update::spawn_startup_auto_update(&app_handle, true);
                        app.manage(StartupRecoveryState(std::sync::Mutex::new(Some(recovery))));
                        return Ok(());
                    }
                    None => return Err(std::io::Error::other(error).into()),
                },
            }
            app.manage(SchedulerState::new());
            #[cfg(feature = "devtools")]
            app.manage(NotificationObserverState::new());

            setup_notifications(&app_handle);

            let initial_theme = {
                let db = app.state::<AppDbConnection>();
                let mut conn = db.0.lock().unwrap();
                settings::theme_hint(&mut conn)
            };
            settings::apply_native_theme(&app_handle, &initial_theme);
            settings::spawn_theme_watcher(&app_handle);

            #[cfg(all(
                feature = "ui-plane",
                any(target_os = "linux", target_os = "macos", target_os = "windows"),
                not(debug_assertions)
            ))]
            if std::env::var_os("FLATPAK_ID").is_none() {
                if let Err(e) = app_handle.autolaunch().enable() {
                    eprintln!("[autostart] enable failed: {e}");
                }
            }

            let db_state = app.state::<AppDbConnection>();
            reconciler::run(&app_handle, &db_state);

            resume_watcher::spawn(&app_handle);

            let data_dir = app_data_dir(&app_handle);
            let dc_state = DeviceConnectionState::from_app_data_dir(&data_dir);
            tauri::async_runtime::spawn(run_ws_server(dc_state.clone(), dc_state.db_path.clone()));
            app.manage(dc_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_spaces,
            backup_export,
            backup_preflight_import,
            backup_apply_import,
            create_space,
            update_space,
            delete_space,
            get_quests,
            get_active_focus,
            create_quest,
            set_focus,
            update_quest,
            delete_quest,
            delete_quest_series,
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
            get_auto_update_enabled,
            set_auto_update_enabled,
            startup_auto_update_supported,
            get_theme_mode,
            set_theme_mode,
            sync_native_theme,
            startup_recovery,
            notification_action,
            notification_tap,
            #[cfg(feature = "devtools")]
            e2e_list_notification_events,
            #[cfg(feature = "devtools")]
            e2e_clear_notification_events,
            #[cfg(feature = "devtools")]
            e2e_dispatch_notification_action,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(all(test, feature = "ui-plane"))]
mod startup_recovery_tests {
    use super::unsupported_schema_startup_recovery;

    #[test]
    fn unsupported_schema_error_selects_update_required_recovery() {
        let recovery = unsupported_schema_startup_recovery(
            "database schema is not supported by this Fini binary. Update required (AppImage)."
                .to_string(),
        )
        .expect("unsupported schema should route to startup recovery");

        assert_eq!(recovery.kind, "update-required");
        assert_eq!(recovery.title, "Update required");
        assert!(recovery.message.contains("Update required (AppImage)"));
    }

    #[test]
    fn unrelated_database_error_remains_startup_failure() {
        assert!(
            unsupported_schema_startup_recovery("failed to open database".to_string()).is_none()
        );
    }
}
