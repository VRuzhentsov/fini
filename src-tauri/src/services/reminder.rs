use diesel::sqlite::SqliteConnection;
#[cfg(any(feature = "ui-plane", test))]
use tauri::{AppHandle, State};

#[cfg(any(feature = "ui-plane", test))]
use crate::models::Space;
use crate::models::{CreateReminderInput, Quest, Reminder};
use crate::repositories::ReminderRepository;

#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::DbPool;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::notification;
#[cfg(any(feature = "ui-plane", test))]
use crate::repositories::QuestRepository;

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
        let (_, space) = QuestRepository::new(self.repository.conn).load_quest_and_space(&quest.id)?;
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
            QuestRepository::new(self.repository.conn).load_quest_and_space(&input.quest_id)?;
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
            QuestRepository::new(self.repository.conn).load_quest_and_space(&existing.quest_id)?;
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

/// Keeps reminders in step with the quests, on the quest events of the bus
/// (docs/db.md, rule 7).
#[cfg(any(feature = "ui-plane", test))]
impl ReminderService<'_> {
    /// Hears the event bus until it closes. Started once, at app start.
    pub async fn follow_quest_events(
        mut events: tokio::sync::broadcast::Receiver<crate::models::Event>,
        app: AppHandle,
        pool: DbPool,
    ) {
        use crate::models::Event;
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match events.recv().await {
                Ok(Event::QuestChanged { quest }) => tokio::task::block_in_place(|| {
                    if let Err(e) = ReminderService::new(&mut pool.connection()).keep_in_step(&app, &quest.id) {
                        log::warn!("[reminder] could not keep the reminder of {} in step: {e}", quest.id);
                    }
                }),
                Ok(Event::QuestDeleted { reminders, .. }) => tokio::task::block_in_place(|| {
                    let mut conn = pool.connection();
                    let mut service = ReminderService::new(&mut conn);
                    for reminder in &reminders {
                        service.cancel_scheduled(&app, reminder);
                        notification::cancel_snooze_with_conn(service.repository.conn, &app, &reminder.id);
                    }
                }),
                Ok(_) => {}
                Err(RecvError::Lagged(missed)) => {
                    log::warn!("[reminder] missed {missed} events; the reconciler puts reminders in step at the next start")
                }
                Err(RecvError::Closed) => return,
            }
        }
    }

    /// Puts the reminder of one quest in step with it: a one-time quest that
    /// is active with a date has one; a repeating quest has one for its next
    /// open day from today (ADR-0012 D9); any other quest has none.
    pub fn keep_in_step(&mut self, app: &AppHandle, quest_id: &str) -> Result<(), String> {
        let mut quests = QuestRepository::new(self.repository.conn);
        let target = if quests.is_repeating(quest_id)? {
            quests.reminder_occurrence(quest_id)?
        } else {
            let quest = quests.get(quest_id)?;
            (quest.status == "active" && quest.due.is_some()).then_some(quest)
        };
        match target {
            Some(quest) => self.reconcile_with_notifications_for_quest(app, &quest),
            None => self.delete_for_quest_with_notifications(app, quest_id),
        }
    }
}


// ── Tauri commands ───────────────────────────────────────────────────────────

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_reminders(
    state: State<DbPool>,
    quest_id: String,
) -> Result<Vec<Reminder>, String> {
    let mut conn = state.get()?;
    ReminderService::new(&mut conn).list_for_quest(&quest_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_reminder(
    app: AppHandle,
    state: State<DbPool>,
    input: CreateReminderInput,
) -> Result<Reminder, String> {
    let mut conn = state.get()?;
    ReminderService::new(&mut conn).create_with_notifications_for_quest(&app, input)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_reminder(
    app: AppHandle,
    state: State<DbPool>,
    id: String,
    input: crate::models::UpdateReminderInput,
) -> Result<Reminder, String> {
    let mut conn = state.get()?;
    ReminderService::new(&mut conn).update_with_notifications_for_reminder(&app, &id, input)
}

#[cfg(any(feature = "ui-plane", test))]
/// Cancel all scheduled OS notifications for every reminder belonging to a quest.
/// Call this when a quest is completed, abandoned, or deleted.
#[tauri::command]
pub fn cancel_quest_notifications(
    app: AppHandle,
    state: State<DbPool>,
    quest_id: String,
) -> Result<(), String> {
    let mut conn = state.get()?;
    ReminderService::new(&mut conn).cancel_for_quest_notifications(&app, &quest_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_reminder(
    app: AppHandle,
    state: State<DbPool>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.get()?;
    ReminderService::new(&mut conn).delete_with_notifications(&app, &id)
}
