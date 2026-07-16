use diesel::sqlite::SqliteConnection;
#[cfg(any(feature = "ui-plane", test))]
use tauri::{AppHandle, State};

#[cfg(any(feature = "ui-plane", test))]
use crate::models::Space;
use crate::models::{CreateReminderInput, Quest, Reminder};
use crate::repositories::reminder::ReminderRepository;

#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::notification;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::quest::QuestService;

/// Owns reminder reconciliation after quest persistence succeeds.
pub struct ReminderService<'a> {
    repository: ReminderRepository<'a>,
}

impl<'a> ReminderService<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self {
            repository: ReminderRepository::new(conn),
        }
    }

    pub fn reconcile(&mut self, quest: &Quest) -> Result<(), String> {
        if quest.status == "active" && quest.due.is_some() {
            self.upsert_for_quest(quest).map(|_| ())
        } else {
            self.repository.delete_for_quest(&quest.id)
        }
    }

    pub fn upsert_for_quest(&mut self, quest: &Quest) -> Result<Reminder, String> {
        let due_at_utc = quest
            .due
            .as_deref()
            .and_then(|due| {
                crate::services::due_time::compute_fire_utc(due, quest.due_time.as_deref())
            })
            .map(|time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        self.repository.upsert_for_quest(&quest.id, due_at_utc)
    }

    pub fn delete_for_quest(&mut self, quest_id: &str) -> Result<(), String> {
        self.repository.delete_for_quest(quest_id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn get(&mut self, id: &str) -> Result<Reminder, String> {
        self.repository.get(id)
    }

    pub fn list_for_quest(&mut self, quest_id: &str) -> Result<Vec<Reminder>, String> {
        self.repository.list_for_quest(quest_id)
    }

    pub fn create(&mut self, input: CreateReminderInput) -> Result<Reminder, String> {
        self.repository.create(input)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update(
        &mut self,
        id: &str,
        input: crate::models::UpdateReminderInput,
    ) -> Result<Reminder, String> {
        self.repository.update(id, input)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        self.repository.delete(id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn cancel_scheduled(&mut self, app: &AppHandle, reminder: &Reminder) {
        if let Some(notification_id) = &reminder.scheduled_notification_id {
            notification::cancel_reminder(app, notification_id);
        }
        notification::cancel_in_process(app, &reminder.id);
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn reconcile_with_notifications(
        &mut self,
        app: &AppHandle,
        quest: &Quest,
        space: &Space,
    ) -> Result<(), String> {
        if quest.status != "active" || quest.due.is_none() {
            return Ok(());
        }
        if let Some(existing) = self.repository.get_for_quest(&quest.id)? {
            self.cancel_scheduled(app, &existing);
        }
        let reminder = self.upsert_for_quest(quest)?;
        let notification_id = notification::schedule_reminder(app, &reminder, quest, space);
        self.repository
            .set_scheduled_notification_id(&reminder.id, notification_id.as_deref())?;
        Ok(())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn reconcile_with_notifications_for_quest(
        &mut self,
        app: &AppHandle,
        quest: &Quest,
    ) -> Result<(), String> {
        let (_, space) = QuestService::new(self.repository.conn).get_with_space(&quest.id)?;
        self.reconcile_with_notifications(app, quest, &space)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn cancel_for_quest_notifications(
        &mut self,
        app: &AppHandle,
        quest_id: &str,
    ) -> Result<(), String> {
        for reminder in self.repository.list_for_quest(quest_id)? {
            self.cancel_scheduled(app, &reminder);
        }
        Ok(())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn delete_for_quest_with_notifications(
        &mut self,
        app: &AppHandle,
        quest_id: &str,
    ) -> Result<(), String> {
        let reminders = self.repository.list_for_quest(quest_id)?;
        for reminder in &reminders {
            self.cancel_scheduled(app, reminder);
            notification::cancel_snooze_with_conn(self.repository.conn, app, &reminder.id);
        }
        self.repository.delete_for_quest(quest_id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn create_with_notifications(
        &mut self,
        app: &AppHandle,
        input: CreateReminderInput,
        quest: &Quest,
        space: &Space,
    ) -> Result<Reminder, String> {
        let reminder = self.repository.create(input)?;
        if reminder.due_at_utc.is_none() {
            return Ok(reminder);
        }
        let notification_id = notification::schedule_reminder(app, &reminder, quest, space);
        self.repository
            .set_scheduled_notification_id(&reminder.id, notification_id.as_deref())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn create_with_notifications_for_quest(
        &mut self,
        app: &AppHandle,
        input: CreateReminderInput,
    ) -> Result<Reminder, String> {
        let (quest, space) =
            QuestService::new(self.repository.conn).get_with_space(&input.quest_id)?;
        self.create_with_notifications(app, input, &quest, &space)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update_with_notifications(
        &mut self,
        app: &AppHandle,
        id: &str,
        input: crate::models::UpdateReminderInput,
        quest: &Quest,
        space: &Space,
    ) -> Result<Reminder, String> {
        let existing = self.repository.get(id)?;
        self.cancel_scheduled(app, &existing);
        let updated = self.update(id, input)?;
        let notification_id = updated
            .due_at_utc
            .as_ref()
            .and_then(|_| notification::schedule_reminder(app, &updated, quest, space));
        self.repository
            .set_scheduled_notification_id(&updated.id, notification_id.as_deref())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update_with_notifications_for_reminder(
        &mut self,
        app: &AppHandle,
        id: &str,
        input: crate::models::UpdateReminderInput,
    ) -> Result<Reminder, String> {
        let existing = self.get(id)?;
        let (quest, space) =
            QuestService::new(self.repository.conn).get_with_space(&existing.quest_id)?;
        self.update_with_notifications(app, id, input, &quest, &space)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn delete_with_notifications(&mut self, app: &AppHandle, id: &str) -> Result<(), String> {
        if let Ok(reminder) = self.repository.get(id) {
            self.cancel_scheduled(app, &reminder);
        }
        self.repository.delete(id)
    }
}

#[cfg(any(feature = "ui-plane", test))]
/// Upsert a Reminder row and schedule (or reschedule) the OS notification.
pub fn upsert_reminder_for_quest(
    conn: &mut SqliteConnection,
    app: &AppHandle,
    quest: &Quest,
) -> Result<(), String> {
    ReminderService::new(conn).reconcile_with_notifications_for_quest(app, quest)
}

#[cfg(any(feature = "ui-plane", test))]
/// Delete all Reminder rows for a quest and cancel their OS notifications.
pub fn delete_reminder_for_quest(
    conn: &mut SqliteConnection,
    app: &AppHandle,
    quest_id: &str,
) -> Result<(), String> {
    ReminderService::new(conn).delete_for_quest_with_notifications(app, quest_id)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_reminders(
    state: State<AppDbConnection>,
    quest_id: String,
) -> Result<Vec<Reminder>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    ReminderService::new(&mut conn).list_for_quest(&quest_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_reminder(
    app: AppHandle,
    state: State<AppDbConnection>,
    input: CreateReminderInput,
) -> Result<Reminder, String> {
    let mut conn = state.inner().0.lock().unwrap();
    ReminderService::new(&mut conn).create_with_notifications_for_quest(&app, input)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_reminder(
    app: AppHandle,
    state: State<AppDbConnection>,
    id: String,
    input: crate::models::UpdateReminderInput,
) -> Result<Reminder, String> {
    let mut conn = state.inner().0.lock().unwrap();
    ReminderService::new(&mut conn).update_with_notifications_for_reminder(&app, &id, input)
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
    ReminderService::new(&mut conn).cancel_for_quest_notifications(&app, &quest_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_reminder(
    app: AppHandle,
    state: State<AppDbConnection>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    ReminderService::new(&mut conn).delete_with_notifications(&app, &id)
}
