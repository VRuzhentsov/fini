use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::{CreateReminderInput, Reminder, UpdateReminderInput};
use crate::schema::reminders;
use crate::services::db::utc_now;

/// Diesel persistence boundary for local reminder rows.
pub struct ReminderRepository<'a> {
    pub(crate) conn: &'a mut SqliteConnection,
}

impl<'a> ReminderRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn get(&mut self, id: &str) -> Result<Reminder, String> {
        reminders::table
            .find(id)
            .select(Reminder::as_select())
            .first(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn get_for_quest(&mut self, quest_id: &str) -> Result<Option<Reminder>, String> {
        reminders::table
            .filter(reminders::quest_id.eq(quest_id))
            .select(Reminder::as_select())
            .first(self.conn)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn upsert_for_quest(
        &mut self,
        quest_id: &str,
        due_at_utc: Option<String>,
    ) -> Result<Reminder, String> {
        if let Some(existing) = self.get_for_quest(quest_id)? {
            diesel::update(reminders::table.find(&existing.id))
                .set(reminders::due_at_utc.eq(due_at_utc))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
            return self.get(&existing.id);
        }

        let now = utc_now();
        diesel::insert_into(reminders::table)
            .values((
                reminders::quest_id.eq(quest_id),
                reminders::kind.eq("absolute"),
                reminders::due_at_utc.eq(due_at_utc),
                reminders::created_at.eq(now),
            ))
            .returning(Reminder::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn set_scheduled_notification_id(
        &mut self,
        id: &str,
        notification_id: Option<&str>,
    ) -> Result<Reminder, String> {
        diesel::update(reminders::table.find(id))
            .set(reminders::scheduled_notification_id.eq(notification_id))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        self.get(id)
    }

    pub fn delete_for_quest(&mut self, quest_id: &str) -> Result<(), String> {
        diesel::delete(reminders::table.filter(reminders::quest_id.eq(quest_id)))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_for_quest(&mut self, quest_id: &str) -> Result<Vec<Reminder>, String> {
        reminders::table
            .filter(reminders::quest_id.eq(quest_id))
            .select(Reminder::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn create(&mut self, input: CreateReminderInput) -> Result<Reminder, String> {
        diesel::insert_into(reminders::table)
            .values(&input)
            .returning(Reminder::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn update(&mut self, id: &str, input: UpdateReminderInput) -> Result<Reminder, String> {
        diesel::update(reminders::table.find(id))
            .set(input)
            .returning(Reminder::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        diesel::delete(reminders::table.find(id))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
