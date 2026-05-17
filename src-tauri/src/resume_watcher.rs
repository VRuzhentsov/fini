/// Listens for system sleep/wake events and re-runs the reconciler on resume.
/// This catches reminders that fired while the system was suspended and Fini was open.
///
/// Platform mapping (Planify-inspired):
/// - Linux: org.freedesktop.login1.Manager PrepareForSleep signal
/// - macOS/Windows: in-process timers and launch reconciler cover most cases
/// - Android: no-op; AlarmManager handles wake-from-doze itself
use tauri::AppHandle;

#[cfg(target_os = "linux")]
use tauri::Manager;

use crate::services::db::DbState;
use crate::services::reconciler;

pub fn spawn(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    spawn_linux(app);

    // macOS/Windows: Tauri's runtime re-arms tokio timers on resume; the rearm_snoozed_reminders
    // call on next launch covers most cases. A full NSWorkspaceDidWakeNotification /
    // WM_POWERBROADCAST bridge is tracked as a follow-up.
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

#[cfg(target_os = "linux")]
fn spawn_linux(app: &AppHandle) {
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = linux_sleep_watch(app_clone).await {
            eprintln!("[resume_watcher] Linux logind listener failed: {e}");
        }
    });
}

#[cfg(target_os = "linux")]
async fn linux_sleep_watch(app: AppHandle) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;
    use zbus::Connection;

    let conn = Connection::system().await?;

    // Subscribe to org.freedesktop.login1.Manager.PrepareForSleep signal.
    // The bool argument is `true` before sleep, `false` on resume.
    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;

    let mut stream = proxy.receive_signal("PrepareForSleep").await?;

    while let Some(msg) = stream.next().await {
        let body = msg.body();
        let is_sleeping = body.deserialize::<(bool,)>().map(|(b,)| b).unwrap_or(true);
        if !is_sleeping {
            // System just resumed — re-run reconciler to catch up missed reminder fires.
            let db = match app.try_state::<DbState>() {
                Some(s) => s,
                None => continue,
            };
            reconciler::run(&app, &db);
        }
    }

    Ok(())
}
