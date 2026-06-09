use std::collections::HashMap;
use std::sync::Mutex;

use diesel::prelude::*;
#[cfg(feature = "devtools")]
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
#[cfg(target_os = "android")]
use tauri_plugin_notification::Channel;
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;

use crate::models::{InsertNotificationSnooze, NotificationSnooze, Quest, Reminder, Space};
use crate::schema::notification_snoozes;
use crate::services::db::{utc_now, AppDbConnection};

#[cfg(not(target_os = "linux"))]
const CHANNEL_ID: &str = "fini.reminders";

#[allow(dead_code)]
const ACTION_TYPE_REMINDER: &str = "reminder";
const ACTION_COMPLETE: &str = "complete";
const ACTION_SNOOZE_30M: &str = "snooze_30m";
const ACTION_SNOOZE_1D: &str = "snooze_1d";

// Tauri event emitted when a notification fires while the app is in the foreground.
pub const FOREGROUND_FIRE_EVENT: &str = "notification://foreground-fire";
// Tauri event emitted when the user taps a reminder notification.
pub const TAP_EVENT: &str = "notification://tap";

/// Compute the UTC fire time from quest due fields using the local wall-clock timezone.
/// If `due_time` is None, defaults to 09:00 local.
pub fn compute_fire_utc(
    due: &str,
    due_time: Option<&str>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    crate::services::due_time::compute_fire_utc(due, due_time)
}

/// In-process timer handles keyed by reminder ID (desktop only).
pub struct SchedulerState(pub Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>);

impl SchedulerState {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

#[cfg(feature = "devtools")]
#[derive(Clone, Serialize)]
pub struct NotificationEvent {
    pub phase: String,
    pub delivery_path: String,
    pub reminder_id: String,
    pub quest_id: String,
    pub body: String,
    pub due_at_utc: Option<String>,
    pub scheduled_notification_id: Option<String>,
}

#[cfg(feature = "devtools")]
pub struct NotificationObserverState(pub Mutex<Vec<NotificationEvent>>);

#[cfg(feature = "devtools")]
impl NotificationObserverState {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }
}

#[cfg(feature = "devtools")]
fn record_notification_event(app: &AppHandle, event: NotificationEvent) {
    if let Some(state) = app.try_state::<NotificationObserverState>() {
        state.0.lock().unwrap().push(event);
    }
}

#[cfg(feature = "devtools")]
#[tauri::command]
pub fn e2e_list_notification_events(app: AppHandle) -> Result<Vec<NotificationEvent>, String> {
    let state = app
        .try_state::<NotificationObserverState>()
        .ok_or_else(|| "notification observer is not available".to_string())?;
    let events = state.0.lock().unwrap().clone();
    Ok(events)
}

#[cfg(feature = "devtools")]
#[tauri::command]
pub fn e2e_clear_notification_events(app: AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<NotificationObserverState>()
        .ok_or_else(|| "notification observer is not available".to_string())?;
    state.0.lock().unwrap().clear();
    Ok(())
}

/// Directly dispatch a notification action without going through the OS notification layer.
/// Only available in devtools builds to simulate user action button clicks.
#[cfg(feature = "devtools")]
#[tauri::command]
pub fn e2e_dispatch_notification_action(
    app: AppHandle,
    reminder_id: String,
    action_id: String,
) -> Result<(), String> {
    dispatch_action(&app, &action_id, &reminder_id);
    Ok(())
}

/// Register the notification channel and action types on app startup.
pub fn setup_notifications(app: &AppHandle) {
    setup_notification_channel(app);
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

/// True when the app window is currently visible and focused.
fn is_app_foreground(app: &AppHandle) -> bool {
    app.webview_windows()
        .values()
        .any(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
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
            .body(&body)
            .icon("ic_stat_fini")
            .large_icon("ic_launcher")
            .icon_color("#0057B7")
            .extra("reminder_id", &reminder.id)
            .action_type_id(ACTION_TYPE_REMINDER)
            .schedule(tauri_plugin_notification::Schedule::At {
                date: due,
                repeating: false,
                allow_while_idle: true,
            })
            .show();

        match result {
            Ok(()) => {
                #[cfg(feature = "devtools")]
                record_notification_event(
                    app,
                    NotificationEvent {
                        phase: "scheduled".to_string(),
                        delivery_path: "mobile_plugin".to_string(),
                        reminder_id: reminder.id.clone(),
                        quest_id: quest.id.clone(),
                        body: body.clone(),
                        due_at_utc: Some(due_utc.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                        scheduled_notification_id: Some(notif_id.to_string()),
                    },
                );
                Some(notif_id.to_string())
            }
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
            return None;
        }

        let body = format!("{} · {}", quest.title, space.name);
        let app_clone = app.clone();
        let reminder_id = reminder.id.clone();
        let reminder_id_key = reminder_id.clone();
        let quest_id = quest.id.clone();
        let body_for_timer = body.clone();

        let handle = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
            show_now(&app_clone, &reminder_id, &quest_id, &body_for_timer);
        });

        if let Some(state) = app.try_state::<SchedulerState>() {
            state.0.lock().unwrap().insert(reminder_id_key, handle);
        }

        #[cfg(feature = "devtools")]
        record_notification_event(
            app,
            NotificationEvent {
                phase: "scheduled".to_string(),
                delivery_path: "desktop_in_process".to_string(),
                reminder_id: reminder.id.clone(),
                quest_id: quest.id.clone(),
                body,
                due_at_utc: Some(due_utc.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                scheduled_notification_id: None,
            },
        );

        None
    }
}

/// Fire an immediate notification (used by reconciler for past-due reminders on all platforms).
/// Always shows as an OS notification — bypasses foreground suppression because these reminders
/// fired while the app was closed and the user has not yet seen them.
pub fn fire_immediate(app: &AppHandle, reminder: &Reminder, quest: &Quest, space: &Space) {
    let body = format!("{} · {}", quest.title, space.name);
    #[cfg(target_os = "linux")]
    show_linux(app, &reminder.id, &quest.id, &body);
    #[cfg(not(target_os = "linux"))]
    show_plugin(app, &reminder.id, &quest.id, &body);
}

/// Send a notification immediately, with foreground suppression.
/// When the app window is focused, emits a Tauri event for in-app handling instead.
pub fn show_now(app: &AppHandle, reminder_id: &str, quest_id: &str, body: &str) {
    if is_app_foreground(app) {
        let _ = app.emit(
            FOREGROUND_FIRE_EVENT,
            serde_json::json!({
                "questId": quest_id,
                "reminderId": reminder_id,
                "body": body,
            }),
        );
        return;
    }

    #[cfg(target_os = "linux")]
    show_linux(app, reminder_id, quest_id, body);

    #[cfg(not(target_os = "linux"))]
    show_plugin(app, reminder_id, quest_id, body);
}

/// Linux notification via notify-rust directly, to get action buttons and icon.
/// Runs in spawn_blocking to avoid zbus/tokio conflict.
#[cfg(target_os = "linux")]
fn show_linux(app: &AppHandle, reminder_id: &str, quest_id: &str, body: &str) {
    let app_clone = app.clone();
    let reminder_id_owned = reminder_id.to_string();
    let body_owned = body.to_string();
    #[allow(unused_variables)]
    let quest_id_owned = quest_id.to_string();

    #[cfg(feature = "devtools")]
    let app_for_record = app.clone();
    #[cfg(feature = "devtools")]
    let reminder_id_for_record = reminder_id.to_string();
    #[cfg(feature = "devtools")]
    let body_for_record = body.to_string();

    // std::thread::spawn (not tokio::task::spawn_blocking) so this can be called
    // from non-async contexts (reconciler runs on the main thread at startup).
    // A local single-threaded runtime is created inside the thread so that
    // notify-rust's zbus internals can call Handle::current() successfully.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("notify-rust tokio rt");
        let _enter = rt.enter();

        let mut notif = notify_rust::Notification::new();
        notif
            .summary("Fini")
            .body(&body_owned)
            .icon("fini")
            .hint(notify_rust::Hint::DesktopEntry("Fini".to_string()))
            .hint(notify_rust::Hint::Urgency(notify_rust::Urgency::Critical))
            .action(ACTION_COMPLETE, "Complete")
            .action(ACTION_SNOOZE_30M, "Snooze 30m")
            .action(ACTION_SNOOZE_1D, "Snooze 1d");

        // KDE Plasma 6 ignores the sound-name DBus hint for third-party apps,
        // so play the system notification sound directly via paplay.
        for path in &[
            "/usr/share/sounds/ocean/stereo/button-pressed.oga",
            "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga",
        ] {
            if std::path::Path::new(path).exists() {
                let _ = std::process::Command::new("paplay").arg(path).spawn();
                break;
            }
        }

        match notif.show() {
            Ok(handle) => {
                #[cfg(feature = "devtools")]
                record_notification_event(
                    &app_for_record,
                    NotificationEvent {
                        phase: "delivered".to_string(),
                        delivery_path: "linux_notify_rust".to_string(),
                        reminder_id: reminder_id_for_record.clone(),
                        quest_id: quest_id_owned.clone(),
                        body: body_for_record.clone(),
                        due_at_utc: None,
                        scheduled_notification_id: None,
                    },
                );

                handle.wait_for_action(|action| {
                    #[cfg(feature = "devtools")]
                    record_notification_event(
                        &app_for_record,
                        NotificationEvent {
                            phase: "action".to_string(),
                            delivery_path: "linux_notify_rust".to_string(),
                            reminder_id: reminder_id_for_record.clone(),
                            quest_id: quest_id_owned.clone(),
                            body: body_for_record.clone(),
                            due_at_utc: None,
                            scheduled_notification_id: Some(action.to_string()),
                        },
                    );

                    dispatch_action(&app_clone, action, &reminder_id_owned);
                });
            }
            Err(e) => {
                eprintln!("[notification] Linux notify-rust failed for {reminder_id_owned}: {e}")
            }
        }
    });
}

/// Non-Linux desktop and mobile plugin path.
#[cfg(not(target_os = "linux"))]
fn show_plugin(app: &AppHandle, reminder_id: &str, quest_id: &str, body: &str) {
    let notif_id = notification_id_from_reminder(reminder_id);
    let builder = app
        .notification()
        .builder()
        .id(notif_id)
        .channel_id(CHANNEL_ID)
        .title("Fini")
        .body(body)
        .icon("fini");

    #[cfg(mobile)]
    let builder = builder
        .large_icon("ic_launcher")
        .icon_color("#0057B7")
        .action_type_id(ACTION_TYPE_REMINDER);

    let result = builder.show();

    match result {
        Ok(()) => {
            #[cfg(feature = "devtools")]
            record_notification_event(
                app,
                NotificationEvent {
                    phase: "delivered".to_string(),
                    delivery_path: "desktop_plugin_show".to_string(),
                    reminder_id: reminder_id.to_string(),
                    quest_id: quest_id.to_string(),
                    body: body.to_string(),
                    due_at_utc: None,
                    scheduled_notification_id: Some(notif_id.to_string()),
                },
            );
        }
        Err(e) => eprintln!("[notification] plugin show failed for {reminder_id}: {e}"),
    }
}

/// Dispatch a notification action received from either Linux notify-rust or mobile plugin.
/// action_id: "complete", "snooze_30m", "snooze_1d", "tap"
pub fn dispatch_action(app: &AppHandle, action_id: &str, reminder_id: &str) {
    match action_id {
        ACTION_COMPLETE => {
            // Find the quest for this reminder so we can complete it.
            let quest_id = reminder_quest_id(app, reminder_id);
            if let Some(qid) = quest_id {
                crate::services::quest::complete_quest_for_notification(app, &qid);
            }
        }
        ACTION_SNOOZE_30M => snooze(app, reminder_id, 30),
        ACTION_SNOOZE_1D => snooze(app, reminder_id, 24 * 60),
        "tap" | "__closed" | "default" => {
            // Emit tap event so the frontend can navigate to Focus.
            let quest_id = reminder_quest_id(app, reminder_id).unwrap_or_default();
            let _ = app.emit(TAP_EVENT, serde_json::json!({ "questId": quest_id }));
        }
        _ => {}
    }
}

/// Look up the quest_id for a reminder from the DB.
fn reminder_quest_id(app: &AppHandle, reminder_id: &str) -> Option<String> {
    use crate::schema::reminders;
    let db = app.try_state::<AppDbConnection>()?;
    let mut conn = db.0.lock().unwrap();
    reminders::table
        .find(reminder_id)
        .select(reminders::quest_id)
        .first::<String>(&mut *conn)
        .optional()
        .ok()
        .flatten()
}

/// Snooze a reminder: write a `notification_snoozes` row and arm a new in-process timer.
/// No new reminder row, no FocusHistory event, no SpaceSync (per wiki: snooze is notification-level).
pub fn snooze(app: &AppHandle, reminder_id: &str, minutes: i64) {
    use diesel::prelude::*;

    let fire_at = chrono::Utc::now() + chrono::Duration::minutes(minutes);
    let fire_at_str = fire_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let now_str = utc_now();

    // Persist snooze so it survives app restart.
    if let Some(db) = app.try_state::<AppDbConnection>() {
        let mut conn = db.0.lock().unwrap();
        let row = InsertNotificationSnooze {
            reminder_id: reminder_id.to_string(),
            fire_at_utc: fire_at_str.clone(),
            created_at: now_str,
        };
        if let Err(e) = diesel::insert_into(notification_snoozes::table)
            .values(&row)
            .on_conflict(notification_snoozes::reminder_id)
            .do_update()
            .set((
                notification_snoozes::fire_at_utc.eq(&row.fire_at_utc),
                notification_snoozes::created_at.eq(&row.created_at),
            ))
            .execute(&mut *conn)
        {
            eprintln!("[notification] snooze: persist failed for {reminder_id}: {e}");
        }
    }

    // Cancel the existing in-process timer if any.
    cancel_in_process(app, reminder_id);

    // Arm a new in-process timer for the snoozed fire time.
    let delay_ms = (fire_at - chrono::Utc::now()).num_milliseconds().max(0) as u64;
    let app_clone = app.clone();
    let rid = reminder_id.to_string();

    let handle = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        // Load quest context for the body.
        if let Some(body) = snooze_body(&app_clone, &rid) {
            let quest_id = reminder_quest_id(&app_clone, &rid).unwrap_or_default();
            // Remove the snooze row before firing so the reconciler doesn't double-fire.
            remove_snooze(&app_clone, &rid);
            show_now(&app_clone, &rid, &quest_id, &body);
        }
    });

    if let Some(state) = app.try_state::<SchedulerState>() {
        state
            .0
            .lock()
            .unwrap()
            .insert(reminder_id.to_string(), handle);
    }
}

fn snooze_body(app: &AppHandle, reminder_id: &str) -> Option<String> {
    use crate::schema::{quests, reminders, spaces};
    let db = app.try_state::<AppDbConnection>()?;
    let mut conn = db.0.lock().unwrap();
    let quest_id: String = reminders::table
        .find(reminder_id)
        .select(reminders::quest_id)
        .first(&mut *conn)
        .ok()?;
    let (title, space_id): (String, String) = quests::table
        .find(&quest_id)
        .select((quests::title, quests::space_id))
        .first(&mut *conn)
        .ok()?;
    let space_name: String = spaces::table
        .find(&space_id)
        .select(spaces::name)
        .first(&mut *conn)
        .ok()?;
    Some(format!("{title} · {space_name}"))
}

fn remove_snooze(app: &AppHandle, reminder_id: &str) {
    use diesel::prelude::*;
    if let Some(db) = app.try_state::<AppDbConnection>() {
        let mut conn = db.0.lock().unwrap();
        let _ = diesel::delete(notification_snoozes::table.find(reminder_id)).execute(&mut *conn);
    }
}

/// Re-arm in-process timers for snoozed reminders found in the DB. Called by the reconciler.
pub fn rearm_snoozed_reminders(app: &AppHandle) {
    use diesel::prelude::*;
    let db = match app.try_state::<AppDbConnection>() {
        Some(s) => s,
        None => return,
    };
    let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut conn = db.0.lock().unwrap();

    let snoozed: Vec<NotificationSnooze> = match notification_snoozes::table
        .select(NotificationSnooze::as_select())
        .load(&mut *conn)
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[notification] rearm: query failed: {e}");
            return;
        }
    };

    let past_due: Vec<&NotificationSnooze> = snoozed
        .iter()
        .filter(|s| s.fire_at_utc <= now_str)
        .collect();
    let future: Vec<&NotificationSnooze> =
        snoozed.iter().filter(|s| s.fire_at_utc > now_str).collect();

    // Fire past-due snoozed notifications immediately.
    let ids_to_fire: Vec<String> = past_due.iter().map(|s| s.reminder_id.clone()).collect();
    // Release lock before calling show_now.
    drop(conn);

    for reminder_id in &ids_to_fire {
        remove_snooze(app, reminder_id);
        if let Some(body) = snooze_body(app, reminder_id) {
            let quest_id = reminder_quest_id(app, reminder_id).unwrap_or_default();
            show_now(app, reminder_id, &quest_id, &body);
        }
    }

    // Re-arm in-process timers for future snoozes.
    for s in future {
        let fire_at = match s.fire_at_utc.parse::<chrono::DateTime<chrono::Utc>>() {
            Ok(dt) => dt,
            Err(_) => continue,
        };
        let delay_ms = (fire_at - chrono::Utc::now()).num_milliseconds().max(0) as u64;
        let app_clone = app.clone();
        let rid = s.reminder_id.clone();
        let rid_key = rid.clone();
        let handle = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            if let Some(body) = snooze_body(&app_clone, &rid) {
                let quest_id = reminder_quest_id(&app_clone, &rid).unwrap_or_default();
                remove_snooze(&app_clone, &rid);
                show_now(&app_clone, &rid, &quest_id, &body);
            }
        });
        if let Some(state) = app.try_state::<SchedulerState>() {
            state.0.lock().unwrap().insert(rid_key, handle);
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

/// Cancel the DB snooze record using a connection the caller already holds.
/// Use this instead of `cancel_snooze` when the AppDbConnection mutex is already locked
/// on the current thread to avoid a deadlock.
pub fn cancel_snooze_with_conn(
    conn: &mut diesel::sqlite::SqliteConnection,
    app: &AppHandle,
    reminder_id: &str,
) {
    use diesel::prelude::*;
    let _ = diesel::delete(notification_snoozes::table.find(reminder_id)).execute(conn);
    cancel_in_process(app, reminder_id);
}
