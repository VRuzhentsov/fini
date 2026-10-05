//! Fini's handle on `tauri-plugin-ble-gatt` on Android.
//!
//! The plugin ships ble-gatt's Kotlin bridge, builds the Android backend
//! after the Activity is up, and asks for the Bluetooth runtime permissions
//! (ble-gatt ADR-0007 D9). Its API hangs off the `AppHandle`, which the
//! Bluetooth channel and the permission checks deep in the communication
//! services do not carry, so `lib.rs` installs the handle here once at
//! setup.

use std::sync::{Arc, OnceLock};

use ble_gatt::Backend;
use tauri::plugin::PermissionState;
use tauri::AppHandle;
use tauri_plugin_ble_gatt::BleGattExt;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Called once from `.setup()`, after the plugin is registered.
pub fn install(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

fn app() -> Result<&'static AppHandle, String> {
    APP.get()
        .ok_or_else(|| "tauri-plugin-ble-gatt is not installed yet".to_string())
}

/// The plugin's backend: one for the process, built on first use.
pub fn backend() -> Result<Arc<dyn Backend>, String> {
    Ok(app()?.ble_gatt().backend())
}

/// Whether the Bluetooth runtime permissions are granted. Fails closed: an
/// unanswerable check reads as not granted.
pub fn permission_granted() -> bool {
    match app().and_then(|app| app.ble_gatt().permission_state().map_err(|err| err.to_string())) {
        Ok(state) => state == PermissionState::Granted,
        Err(err) => {
            eprintln!("[ble-plugin] permission check failed, treating as not granted: {err}");
            false
        }
    }
}

/// Opens the system permission dialog if the permissions are not granted
/// yet. Returns at once; the answer arrives later, so callers poll
/// [`permission_granted`]. Only from a user action: Android allows asking
/// for Nearby Devices from a click, not from a background tick.
pub fn request_permission() {
    let Ok(app) = app() else {
        eprintln!("[ble-plugin] not installed, not requesting the Bluetooth permission");
        return;
    };
    // `request_permissions` blocks until the person answers the dialog.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(err) = app.ble_gatt().request_permissions() {
            eprintln!("[ble-plugin] requesting the Bluetooth permission failed: {err}");
        }
    });
}
