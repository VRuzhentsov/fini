use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;
#[cfg(target_os = "android")]
use tauri_plugin_notification::Channel;

use crate::models::{Quest, Reminder, Space};

#[cfg(not(target_os = "linux"))]
const CHANNEL_ID: &str = "fini.reminders";

/// Compute the UTC fire time from quest due fields using the local wall-clock timezone.
/// If `due_time` is None, defaults to 09:00 local.
pub fn compute_fire_utc(due: &str, due_time: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
    let date = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
    let time = match due_time {
        Some(t) => NaiveTime::parse_from_str(t, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(t, "%H:%M:%S"))
            .ok()?,
        None => NaiveTime::from_hms_opt(9, 0, 0)?,
    };
    let naive = NaiveDateTime::new(date, time);
    Local.from_local_datetime(&naive).single().map(|dt| dt.with_timezone(&chrono::Utc))
}

/// In-process timer handles keyed by reminder ID (desktop only).
pub struct SchedulerState(pub Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>);

impl SchedulerState {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

/// Register the notification channel on Android (no-op on other platforms).
pub fn setup_notification_channel(app: &AppHandle) {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_notification::Importance;
        let channel = Channel::builder(CHANNEL_ID, "Reminders")
            .importance(Importance::Default)
            .build();
        if let Err(e) = app.notification().create_channel(channel) {
            eprintln!("[notification] channel setup failed: {e}");
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

/// Derive a stable i32 notification ID from a reminder UUID string.
#[cfg(not(target_os = "linux"))]
fn notification_id_from_reminder(reminder_id: &str) -> i32 {
    let bytes = reminder_id.as_bytes();
    let mut h: i32 = 0x1505i32;
    for &b in bytes {
        h = h.wrapping_mul(31).wrapping_add(b as i32);
    }
    h
}

/// Schedule an OS notification for a reminder.
/// Mobile: uses AlarmManager via tauri-plugin-notification, returns the notification ID.
/// Desktop: spawns an in-process tokio timer; returns None (no OS-level handle needed).
pub fn schedule_reminder(
    app: &AppHandle,
    reminder: &Reminder,
    quest: &Quest,
    space: &Space,
) -> Option<String> {
    #[cfg(mobile)]
    {
        let due_utc = compute_fire_utc(quest.due.as_deref()?, quest.due_time.as_deref())?;
        let due_ts = due_utc.timestamp();
        let due = time::OffsetDateTime::from_unix_timestamp(due_ts).ok()?;

        let notif_id = notification_id_from_reminder(&reminder.id);
        let body = format!("{} · {}", quest.title, space.name);

        let result = app
            .notification()
            .builder()
            .id(notif_id)
            .channel_id(CHANNEL_ID)
            .title("Fini")
            .body(body)
            .schedule(tauri_plugin_notification::Schedule::At {
                date: due,
                repeating: false,
                allow_while_idle: true,
            })
            .show();

        match result {
            Ok(()) => Some(notif_id.to_string()),
            Err(e) => {
                eprintln!("[notification] schedule failed for {}: {e}", reminder.id);
                None
            }
        }
    }
    #[cfg(not(mobile))]
    {
        let due_utc = compute_fire_utc(quest.due.as_deref()?, quest.due_time.as_deref())?;
        let now = chrono::Utc::now();
        let delay_ms = (due_utc - now).num_milliseconds();
        if delay_ms <= 0 {
            // Past-due: reconciler handles this on next launch.
            return None;
        }

        let body = format!("{} · {}", quest.title, space.name);
        let app_clone = app.clone();
        let reminder_id = reminder.id.clone();
        let reminder_id_key = reminder_id.clone();

        let handle = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
            show_now(&app_clone, &reminder_id, &body);
        });

        if let Some(state) = app.try_state::<SchedulerState>() {
            state.0.lock().unwrap().insert(reminder_id_key, handle);
        }

        None
    }
}

/// Fire an immediate notification (used by reconciler for past-due reminders on all platforms).
pub fn fire_immediate(app: &AppHandle, reminder: &Reminder, quest: &Quest, space: &Space) {
    if quest.due.is_none() {
        return;
    }
    let body = format!("{} · {}", quest.title, space.name);
    show_now(app, &reminder.id, &body);
}

/// Send a notification immediately. On Linux uses notify-send to avoid
/// zbus/block_on conflicts inside tokio tasks.
fn show_now(app: &AppHandle, reminder_id: &str, body: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        if let Err(e) = std::process::Command::new("notify-send")
            .args(["-a", "Fini", "Fini", body])
            .spawn()
        {
            eprintln!("[notification] notify-send failed for {reminder_id}: {e}");
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let notif_id = notification_id_from_reminder(reminder_id);
        if let Err(e) = app
            .notification()
            .builder()
            .id(notif_id)
            .channel_id(CHANNEL_ID)
            .title("Fini")
            .body(body.to_string())
            .show()
        {
            eprintln!("[notification] fire_immediate failed for {reminder_id}: {e}");
        }
    }
}

/// Cancel a previously scheduled OS notification (mobile: cancels AlarmManager alarm).
pub fn cancel_reminder(app: &AppHandle, scheduled_notification_id: &str) {
    #[cfg(mobile)]
    {
        if let Ok(id) = scheduled_notification_id.parse::<i32>() {
            if let Err(e) = app.notification().cancel(vec![id]) {
                eprintln!("[notification] cancel failed for id {id}: {e}");
            }
        }
    }
    #[cfg(not(mobile))]
    let _ = (app, scheduled_notification_id);
}

/// Abort the in-process timer for a reminder (desktop only).
pub fn cancel_in_process(app: &AppHandle, reminder_id: &str) {
    if let Some(state) = app.try_state::<SchedulerState>() {
        if let Some(handle) = state.0.lock().unwrap().remove(reminder_id) {
            handle.abort();
        }
    }
}
