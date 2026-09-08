pub mod models;
#[cfg(not(feature = "ui-plane"))]
extern crate self as tauri;
#[cfg(not(feature = "ui-plane"))]
pub mod async_runtime;
pub mod repositories;
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
    device_connection_discover_bluetooth_candidates, device_connection_discovery_snapshot,
    device_connection_enter_add_mode, device_connection_find_bluetooth_address,
    device_connection_get_identity, device_connection_get_paired_devices,
    device_connection_leave_add_mode, device_connection_pair_accept_request,
    device_connection_pair_acknowledge_request, device_connection_pair_complete_request,
    device_connection_pair_incoming_requests, device_connection_pair_outgoing_completions,
    device_connection_pair_outgoing_updates, device_connection_presence_snapshot,
    device_connection_retry_bluetooth_dial, device_connection_save_paired_device,
    device_connection_send_pair_request, device_connection_send_pair_request_bluetooth,
    device_connection_session_transport, device_connection_set_bluetooth_transport,
    device_connection_set_preferred_transport, device_connection_transport_liveness,
    device_connection_transport_statuses, device_connection_unpair, device_connection_update_last_seen,
    DeviceConnectionState,
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
    add_checklist_item, create_quest, delete_quest, delete_quest_series, edit_checklist_item,
    get_active_focus, get_checklist_activity, get_quests, get_series_checklist_template,
    remove_checklist_item, reorder_checklist, set_focus, toggle_checklist_item, update_quest,
    update_series_checklist,
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
    space_sync_apply_remote_mappings, space_sync_list_mappings,
    space_sync_resolve_custom_space_mapping, space_sync_status, space_sync_tick,
    space_sync_update_mappings,
};
#[cfg(all(feature = "ui-plane", target_os = "linux"))]
use services::transport::ble;
#[cfg(feature = "ui-plane")]
use services::transport::{sim, tcp_ws};
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

/// Loopback port the devtools control plane listens on for Android debug
/// builds. Matches the tauri-mcp `driver_session` tool's own default port, so
/// `adb forward tcp:9223 tcp:9223` is the only step needed to drive a real
/// device from the host. Desktop builds use a unix socket instead -- see the
/// plugin registration in `run` for why Android can't.
#[cfg(all(feature = "devtools", target_os = "android"))]
const DEVTOOLS_ANDROID_TCP_PORT: u16 = 9223;

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

/// Payload for `SESSION_CHANGED_EVENT` — ADR-0003 Phase 2. `established`
/// distinguishes the two `LifecycleEvent` variants; `kind` is the
/// finer-grained `services::transport::TransportKind` the event itself
/// carries (TcpWs/Sim/Bluetooth), not `device_connection::transport`'s
/// coarser Network/Bluetooth row kind — the frontend doesn't need to
/// interpret it, it's just enough for the listener to log/filter on if it
/// ever wants to.
#[cfg(feature = "ui-plane")]
#[derive(Clone, serde::Serialize)]
struct SessionChangedEvent {
    peer_device_id: String,
    kind: services::transport::TransportKind,
    established: bool,
}

#[cfg(feature = "ui-plane")]
const SESSION_CHANGED_EVENT: &str = "device-connection://session-changed";

/// Forwards `DeviceConnectionState::subscribe_lifecycle()` to the frontend
/// as they happen, so a session's connect/disconnect reaches the UI faster
/// than the next `TRANSPORT_STATUS_POLL_INTERVAL_MS` poll (`device.ts`
/// still polls too — this is a latency improvement, not the sole source of
/// truth, so a missed/dropped event here self-heals within that poll
/// window instead of staying wrong indefinitely). See ADR-0003 Phase 2.
#[cfg(feature = "ui-plane")]
async fn forward_session_lifecycle_events(
    mut events: tokio::sync::broadcast::Receiver<services::transport::selection::LifecycleEvent>,
    app: AppHandle,
) {
    use services::transport::selection::LifecycleEvent;

    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            // Lagged: some events were dropped, but the receiver is still
            // live -- keep forwarding what arrives next rather than giving
            // up on the whole subscription. `device.ts`'s poll fallback
            // covers whatever this specific gap missed.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let payload = match event {
            LifecycleEvent::SessionEstablished { peer_device_id, kind } => {
                SessionChangedEvent { peer_device_id, kind, established: true }
            }
            LifecycleEvent::SessionEnded { peer_device_id, kind } => {
                SessionChangedEvent { peer_device_id, kind, established: false }
            }
        };
        let _ = app.emit(SESSION_CHANGED_EVENT, payload);
    }
}

#[cfg(feature = "cli-plane")]
pub fn run_cli() -> i32 {
    services::cli::run(std::env::args().collect())
}

#[cfg(feature = "ui-plane")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        // Registered first, and unconditionally (not gated on
        // `debug_assertions`) -- on Android that guard is exactly what
        // makes a release build silent: `tauri-plugin-log` is what routes
        // records (both `ble_gatt`'s own, and fini's) to logcat there, and
        // a debug-only registration means a release install has no logger
        // installed at all, so every `log::info!`/`warn!`/`error!` call
        // anywhere in the process -- ble-gatt's included -- is silently
        // dropped rather than merely quiet. See ble-gatt's own
        // docs/logging.md, which already assumes this is wired up here.
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: None }),
                ])
                .build(),
        )
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
    // Desktop: a unix socket by default, whose path the actors harness hands
    // us per actor (`TAURI_PLAYWRIGHT_SOCKET`, see
    // specs/e2e/actors/fixtures.ts) -- the harness spawns those processes, so
    // it can pick a path and know when the socket appears.
    //
    // `FINI_DEVTOOLS_TCP_PORT` switches this instance to TCP instead, for a
    // long-lived debug app the harness does *not* spawn: an external actor is
    // addressed by port, exactly like a phone reached over `adb forward`, so
    // a desktop and a device end up equally reachable rather than needing two
    // different connection styles. Note `socket_path` must be cleared, not
    // merely accompanied by `tcp_port` -- see the Android arm below for why.
    #[cfg(all(feature = "devtools", not(target_os = "android")))]
    let builder = {
        let tcp_port = std::env::var("FINI_DEVTOOLS_TCP_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok());
        let config = match tcp_port {
            Some(port) => tauri_plugin_playwright::PluginConfig {
                socket_path: None,
                tcp_port: Some(port),
                window_label: None,
            },
            None => {
                // Falls back to the user's runtime dir rather than a shared
                // world-writable /tmp path: the socket is a control channel
                // into this process, and XDG_RUNTIME_DIR is both the correct
                // place for one and already per-user and per-session. The
                // actors harness always passes an explicit path anyway, so
                // this default only covers a manual run.
                let socket_path = std::env::var("TAURI_PLAYWRIGHT_SOCKET").unwrap_or_else(|_| {
                    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|_| std::env::temp_dir());
                    runtime_dir.join("fini-playwright.sock").to_string_lossy().into_owned()
                });
                tauri_plugin_playwright::PluginConfig::new().socket_path(&socket_path)
            }
        };
        builder.plugin(tauri_plugin_playwright::init_with_config(config))
    };
    // Android: TCP instead. A unix socket is unusable here -- the plugin's
    // default path lives outside the app sandbox, so binding it fails with
    // "unix server error: Permission denied (os error 13)", and there is no
    // way to hand the process a different path anyway (`am start` cannot set
    // environment variables, unlike the desktop actors harness's spawn). TCP
    // on loopback is reachable from a host via `adb forward tcp:9223
    // tcp:9223`, and 9223 is what the tauri-mcp `driver_session` tool
    // already defaults to.
    // `socket_path` must be cleared, not just left at its default alongside
    // `tcp_port`: the plugin's own `server::start` takes the unix branch
    // whenever `socket_path` is `Some` and *returns* from it, so the TCP
    // listener is only ever reached when the socket path is `None`. Android
    // counts as unix, so leaving the default in place means the plugin binds
    // nothing at all here (it just logs the failed unix bind and stops).
    // Built as a struct literal because `PluginConfig::new()` seeds
    // `socket_path` with the desktop default and exposes no setter to unset it.
    #[cfg(all(feature = "devtools", target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_playwright::init_with_config(
        tauri_plugin_playwright::PluginConfig {
            socket_path: None,
            tcp_port: Some(DEVTOOLS_ANDROID_TCP_PORT),
            window_label: None,
        },
    ));
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
            tauri::async_runtime::spawn(tcp_ws::run_server(dc_state.clone(), dc_state.db_path.clone()));
            sim::maybe_spawn_server(dc_state.clone(), dc_state.db_path.clone());
            #[cfg(target_os = "linux")]
            tauri::async_runtime::spawn(ble::run_server(dc_state.clone(), dc_state.db_path.clone()));
            tauri::async_runtime::spawn(forward_session_lifecycle_events(
                dc_state.subscribe_lifecycle(),
                app_handle.clone(),
            ));
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
            add_checklist_item,
            toggle_checklist_item,
            edit_checklist_item,
            remove_checklist_item,
            reorder_checklist,
            update_series_checklist,
            get_checklist_activity,
            get_series_checklist_template,
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
            device_connection_session_transport,
            device_connection_set_bluetooth_transport,
            device_connection_set_preferred_transport,
            device_connection_find_bluetooth_address,
            device_connection_send_pair_request_bluetooth,
            device_connection_discover_bluetooth_candidates,
            device_connection_transport_statuses,
            device_connection_transport_liveness,
            device_connection_retry_bluetooth_dial,
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
