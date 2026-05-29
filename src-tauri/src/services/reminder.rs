use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
#[cfg(any(feature = "ui-plane", test))]
use tauri::{AppHandle, State};

#[cfg(any(feature = "ui-plane", test))]
use crate::models::Space;
#[cfg(any(feature = "ui-plane", test))]
use crate::models::UpdateReminderInput;
use crate::models::{CreateReminderInput, Quest, Reminder};
use crate::schema::reminders;
#[cfg(any(feature = "ui-plane", test))]
use crate::schema::{quests, spaces};
use crate::services::db::utc_now;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::notification;

#[cfg(any(feature = "ui-plane", test))]
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

pub struct ReminderRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> ReminderRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn upsert_for_quest(&mut self, quest: &Quest) -> Result<(), String> {
        let due_str = match quest.due.as_deref() {
            Some(d) => d,
            None => return Ok(()),
        };

        let due_at_utc =
            crate::services::due_time::compute_fire_utc(due_str, quest.due_time.as_deref())
                .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string());

        let existing: Option<String> = reminders::table
            .filter(reminders::quest_id.eq(&quest.id))
            .select(reminders::id)
            .first(self.conn)
            .optional()
            .map_err(|e| e.to_string())?;

        match existing {
            Some(existing_id) => {
                diesel::update(reminders::table.find(&existing_id))
                    .set(reminders::due_at_utc.eq(&due_at_utc))
                    .execute(self.conn)
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
                    .execute(self.conn)
                    .map_err(|e| e.to_string())?;
            }
        }

        Ok(())
    }

    pub fn delete_for_quest(&mut self, quest_id: &str) -> Result<(), String> {
        diesel::delete(reminders::table.filter(reminders::quest_id.eq(quest_id)))
            .execute(self.conn)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_for_quest(&mut self, quest_id: &str) -> Result<Vec<Reminder>, String> {
        reminders::table
            .filter(reminders::quest_id.eq(quest_id))
            .select(Reminder::as_select())
            .load(self.conn)
            .map_err(|e| e.to_string())
    }

    pub fn create(&mut self, input: CreateReminderInput) -> Result<Reminder, String> {
        diesel::insert_into(reminders::table)
            .values(&input)
            .returning(Reminder::as_returning())
            .get_result(self.conn)
            .map_err(|e| e.to_string())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update(&mut self, id: &str, input: UpdateReminderInput) -> Result<Reminder, String> {
        diesel::update(reminders::table.find(id))
            .set(&input)
            .returning(Reminder::as_returning())
            .get_result(self.conn)
            .map_err(|e| e.to_string())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        diesel::delete(reminders::table.find(id))
            .execute(self.conn)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(any(feature = "ui-plane", test))]
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

    ReminderRepository::new(conn).upsert_for_quest(quest)?;

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

#[cfg(any(feature = "ui-plane", test))]
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
        notification::cancel_snooze_with_conn(conn, app, &r.id);
    }

    ReminderRepository::new(conn).delete_for_quest(quest_id)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_reminders(
    state: State<AppDbConnection>,
    quest_id: String,
) -> Result<Vec<Reminder>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    ReminderRepository::new(&mut conn).list_for_quest(&quest_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_reminder(
    app: AppHandle,
    state: State<AppDbConnection>,
    input: CreateReminderInput,
) -> Result<Reminder, String> {
    let notif_id = {
        let mut conn = state.inner().0.lock().unwrap();
        let reminder = ReminderRepository::new(&mut conn).create(input)?;

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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_reminder(
    app: AppHandle,
    state: State<AppDbConnection>,
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
    let updated = ReminderRepository::new(&mut conn).update(&id, input)?;

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

#[cfg(any(feature = "ui-plane", test))]
/// Cancel all scheduled OS notifications for every reminder belonging to a quest.
/// Call this when a quest is completed, abandoned, or deleted.
#[tauri::command]
pub fn cancel_quest_notifications(
    app: AppHandle,
    state: State<AppDbConnection>,
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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_reminder(
    app: AppHandle,
    state: State<AppDbConnection>,
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

    ReminderRepository::new(&mut conn).delete(&id)
}
