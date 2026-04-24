use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use tauri::{AppHandle, State};

use crate::models::{CreateReminderInput, Quest, Reminder, Space, UpdateReminderInput};
use crate::schema::{quests, reminders, spaces};
use crate::services::db::{utc_now, DbState};
use crate::services::notification;

/// Fetch quest + space for notification body building.
fn load_quest_and_space(
    conn: &mut SqliteConnection,
    quest_id: &str,
) -> Result<(Quest, Space), String> {
    let quest = quests::table
        .find(quest_id)
        .select(Quest::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;
    let space = spaces::table
        .find(&quest.space_id)
        .select(Space::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;
    Ok((quest, space))
}

// ── Bridge helpers (called from quest service, not frontend) ─────────────────

/// DB-only upsert: compute due_at_utc from quest fields and upsert the Reminder row.
/// No notification scheduling — callable without AppHandle (e.g. from MCP mode).
pub fn upsert_reminder_db(conn: &mut SqliteConnection, quest: &Quest) -> Result<(), String> {
    let due_str = match quest.due.as_deref() {
        Some(d) => d,
        None => return Ok(()),
    };

    let due_at_utc = notification::compute_fire_utc(due_str, quest.due_time.as_deref())
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string());

    let existing: Option<String> = reminders::table
        .filter(reminders::quest_id.eq(&quest.id))
        .select(reminders::id)
        .first(conn)
        .optional()
        .map_err(|e| e.to_string())?;

    match existing {
        Some(existing_id) => {
            diesel::update(reminders::table.find(&existing_id))
                .set(reminders::due_at_utc.eq(&due_at_utc))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        None => {
            let now = utc_now();
            diesel::insert_into(reminders::table)
                .values((
                    reminders::quest_id.eq(&quest.id),
                    reminders::kind.eq("absolute"),
                    reminders::due_at_utc.eq(&due_at_utc),
                    reminders::created_at.eq(&now),
                ))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// DB-only delete: remove all Reminder rows for a quest.
/// No notification cancellation — callable without AppHandle (e.g. from MCP mode).
pub fn delete_reminder_db(conn: &mut SqliteConnection, quest_id: &str) -> Result<(), String> {
    diesel::delete(reminders::table.filter(reminders::quest_id.eq(quest_id)))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Upsert a Reminder row and schedule (or reschedule) the OS notification.
pub fn upsert_reminder_for_quest(
    conn: &mut SqliteConnection,
    app: &AppHandle,
    quest: &Quest,
) -> Result<(), String> {
    // Cancel any existing notification before the DB update
    if let Some(existing) = reminders::table
        .filter(reminders::quest_id.eq(&quest.id))
        .select(Reminder::as_select())
        .first(conn)
        .optional()
        .map_err(|e| e.to_string())?
    {
        if let Some(ref notif_id) = existing.scheduled_notification_id {
            notification::cancel_reminder(app, notif_id);
        }
        notification::cancel_in_process(app, &existing.id);
    }

    upsert_reminder_db(conn, quest)?;

    let reminder: Reminder = reminders::table
        .filter(reminders::quest_id.eq(&quest.id))
        .select(Reminder::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;

    let space: Space = spaces::table
        .find(&quest.space_id)
        .select(Space::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;

    let new_notif_id = notification::schedule_reminder(app, &reminder, quest, &space);
    if new_notif_id.is_some() {
        diesel::update(reminders::table.find(&reminder.id))
            .set(reminders::scheduled_notification_id.eq(&new_notif_id))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Delete all Reminder rows for a quest and cancel their OS notifications.
pub fn delete_reminder_for_quest(
    conn: &mut SqliteConnection,
    app: &AppHandle,
    quest_id: &str,
) -> Result<(), String> {
    let quest_reminders: Vec<Reminder> = reminders::table
        .filter(reminders::quest_id.eq(quest_id))
        .select(Reminder::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    for r in &quest_reminders {
        if let Some(ref notif_id) = r.scheduled_notification_id {
            notification::cancel_reminder(app, notif_id);
        }
        notification::cancel_in_process(app, &r.id);
    }

    delete_reminder_db(conn, quest_id)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_reminders(state: State<DbState>, quest_id: String) -> Result<Vec<Reminder>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    reminders::table
        .filter(reminders::quest_id.eq(&quest_id))
        .select(Reminder::as_select())
        .load(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_reminder(
    app: AppHandle,
    state: State<DbState>,
    input: CreateReminderInput,
) -> Result<Reminder, String> {
    let notif_id = {
        let mut conn = state.inner().0.lock().unwrap();
        let reminder: Reminder = diesel::insert_into(reminders::table)
            .values(&input)
            .returning(Reminder::as_returning())
            .get_result(&mut *conn)
            .map_err(|e| e.to_string())?;

        if reminder.due_at_utc.is_some() {
            let (quest, space) = load_quest_and_space(&mut conn, &reminder.quest_id)?;
            let id_str = notification::schedule_reminder(&app, &reminder, &quest, &space);
            (reminder.id.clone(), id_str)
        } else {
            (reminder.id.clone(), None)
        }
    };

    let mut conn = state.inner().0.lock().unwrap();
    if let Some(ref id_str) = notif_id.1 {
        diesel::update(reminders::table.find(&notif_id.0))
            .set(reminders::scheduled_notification_id.eq(id_str))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    }

    reminders::table
        .find(&notif_id.0)
        .select(Reminder::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_reminder(
    app: AppHandle,
    state: State<DbState>,
    id: String,
    input: UpdateReminderInput,
) -> Result<Reminder, String> {
    let mut conn = state.inner().0.lock().unwrap();

    // Cancel existing scheduled notification if any
    let existing: Reminder = reminders::table
        .find(&id)
        .select(Reminder::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())?;
    if let Some(ref notif_id) = existing.scheduled_notification_id {
        notification::cancel_reminder(&app, notif_id);
    }
    notification::cancel_in_process(&app, &existing.id);

    // Apply update
    let updated: Reminder = diesel::update(reminders::table.find(&id))
        .set(&input)
        .returning(Reminder::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())?;

    // Reschedule if new time is set
    let new_notif_id = if updated.due_at_utc.is_some() {
        let (quest, space) = load_quest_and_space(&mut conn, &updated.quest_id)?;
        notification::schedule_reminder(&app, &updated, &quest, &space)
    } else {
        None
    };

    // Persist new notification ID (NULL clears old one)
    diesel::update(reminders::table.find(&id))
        .set(reminders::scheduled_notification_id.eq(&new_notif_id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;

    reminders::table
        .find(&id)
        .select(Reminder::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

/// Cancel all scheduled OS notifications for every reminder belonging to a quest.
/// Call this when a quest is completed, abandoned, or deleted.
#[tauri::command]
pub fn cancel_quest_notifications(
    app: AppHandle,
    state: State<DbState>,
    quest_id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    let quest_reminders: Vec<Reminder> = reminders::table
        .filter(reminders::quest_id.eq(&quest_id))
        .select(Reminder::as_select())
        .load(&mut *conn)
        .map_err(|e| e.to_string())?;
    for r in &quest_reminders {
        if let Some(ref notif_id) = r.scheduled_notification_id {
            notification::cancel_reminder(&app, notif_id);
        }
        notification::cancel_in_process(&app, &r.id);
    }
    Ok(())
}

#[tauri::command]
pub fn delete_reminder(
    app: AppHandle,
    state: State<DbState>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();

    // Cancel scheduled notification if any
    if let Ok(reminder) = reminders::table
        .find(&id)
        .select(Reminder::as_select())
        .first(&mut *conn)
    {
        if let Some(ref notif_id) = reminder.scheduled_notification_id {
            notification::cancel_reminder(&app, notif_id);
        }
        notification::cancel_in_process(&app, &reminder.id);
    }

    diesel::delete(reminders::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}
