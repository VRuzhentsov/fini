#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
use tauri_plugin_updater::UpdaterExt;
#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
use url::Url;

#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
const DEFAULT_DESKTOP_UPDATE_ENDPOINT: &str =
    "https://github.com/VRuzhentsov/fini/releases/latest/download/latest.json";
#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
const COMPILED_UPDATE_PUBKEY: Option<&str> = option_env!("FINI_TAURI_UPDATER_PUBKEY");
#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
const DISABLE_AUTO_UPDATE_ENV: &str = "FINI_DISABLE_AUTO_UPDATE";

#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
pub fn spawn_startup_auto_update(app: &tauri::AppHandle) {
    if auto_update_disabled_from_env() {
        eprintln!("[desktop-updater] startup auto-update disabled by {DISABLE_AUTO_UPDATE_ENV}");
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_startup_auto_update(app).await {
            eprintln!("[desktop-updater] startup auto-update skipped: {error}");
        }
    });
}

#[cfg(not(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
)))]
pub fn spawn_startup_auto_update(_app: &tauri::AppHandle) {}

#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
async fn run_startup_auto_update(app: tauri::AppHandle) -> Result<(), String> {
    let config = resolve_desktop_update_config()?;
    let updater = app
        .updater_builder()
        .pubkey(config.pubkey.clone())
        .endpoints(vec![config.endpoint.clone()])
        .map_err(|error| format!("failed to configure updater endpoint: {error}"))?
        .build()
        .map_err(|error| format!("failed to build updater: {error}"))?;

    let Some(update) = updater
        .check()
        .await
        .map_err(|error| format!("failed to check for updates: {error}"))?
    else {
        eprintln!("[desktop-updater] no update available");
        return Ok(());
    };

    eprintln!(
        "[desktop-updater] installing update {} -> {}",
        update.current_version, update.version
    );

    update
        .download_and_install(
            |downloaded, total| {
                if let Some(total) = total {
                    eprintln!("[desktop-updater] downloaded {downloaded}/{total} bytes");
                } else {
                    eprintln!("[desktop-updater] downloaded {downloaded} bytes");
                }
            },
            || eprintln!("[desktop-updater] download finished"),
        )
        .await
        .map_err(|error| format!("failed to install update: {error}"))?;

    eprintln!("[desktop-updater] update installed; restarting");
    app.restart();
}

#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopUpdateConfig {
    endpoint: Url,
    pubkey: String,
}

#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
fn resolve_desktop_update_config() -> Result<DesktopUpdateConfig, String> {
    let endpoint = std::env::var("FINI_DESKTOP_UPDATE_ENDPOINT")
        .or_else(|_| std::env::var("FINI_UPDATE_ENDPOINT"))
        .unwrap_or_else(|_| DEFAULT_DESKTOP_UPDATE_ENDPOINT.to_string())
        .parse::<Url>()
        .map_err(|error| format!("invalid desktop updater endpoint: {error}"))?;
    let pubkey = std::env::var("FINI_DESKTOP_UPDATE_PUBKEY")
        .or_else(|_| std::env::var("FINI_UPDATE_PUBKEY"))
        .ok()
        .or_else(|| COMPILED_UPDATE_PUBKEY.map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "missing Tauri updater public key; set FINI_DESKTOP_UPDATE_PUBKEY, FINI_UPDATE_PUBKEY, or build with FINI_TAURI_UPDATER_PUBKEY".to_string()
        })?;

    Ok(DesktopUpdateConfig { endpoint, pubkey })
}

#[cfg(all(
    feature = "desktop-updater",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(debug_assertions)
))]
fn auto_update_disabled_from_env() -> bool {
    std::env::var(DISABLE_AUTO_UPDATE_ENV)
        .map(|value| auto_update_disabled_value(&value))
        .unwrap_or(false)
}

#[cfg(any(
    test,
    all(
        feature = "desktop-updater",
        any(target_os = "linux", target_os = "macos", target_os = "windows"),
        not(debug_assertions)
    )
))]
fn auto_update_disabled_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_truthy_auto_update_disable_values() {
        for value in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(
                auto_update_disabled_value(value),
                "{value:?} should disable updates"
            );
        }
    }

    #[test]
    fn ignores_falsey_auto_update_disable_values() {
        for value in ["", "0", "false", "no", "off", "anything-else"] {
            assert!(
                !auto_update_disabled_value(value),
                "{value:?} should not disable updates"
            );
        }
    }
}
