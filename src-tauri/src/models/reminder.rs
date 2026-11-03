use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::Quest;
use crate::schema::reminders;

/// A reminder at its quest's due + due_time, minus `mm_offset` minutes.
pub const REMINDER_RELATIVE: &str = "relative";
/// A reminder at a moment of its own (`due_at_utc`).
pub const REMINDER_ABSOLUTE: &str = "absolute";

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone, Debug)]
#[diesel(table_name = reminders)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Reminder {
    pub id: String,
    pub quest_id: String,
    pub kind: String,
    pub mm_offset: Option<i64>,
    /// Stored for an absolute reminder only. A relative reminder's moment is
    /// computed (`moment`); `with_moment` fills it in for the readers.
    pub due_at_utc: Option<String>,
    pub created_at: String,
    pub scheduled_notification_id: Option<String>,
}

impl Reminder {
    /// When the reminder fires. `quest` is the quest as it is shown: for a
    /// repeating quest, its next open day.
    pub fn moment(&self, quest: &Quest) -> Option<DateTime<Utc>> {
        if self.kind == REMINDER_ABSOLUTE {
            return self.due_at_utc.as_deref().and_then(|at| at.parse().ok());
        }
        quest.due_at().map(|due_at| due_at - Duration::minutes(self.mm_offset.unwrap_or(0)))
    }

    /// The reminder as the CLI and the UI read it: `due_at_utc` is its
    /// moment, also when the moment is computed.
    pub fn with_moment(mut self, quest: &Quest) -> Self {
        self.due_at_utc = self.moment(quest).map(|at| at.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        self
    }
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = reminders)]
pub struct CreateReminderInput {
    pub quest_id: String,
    pub kind: String,
    pub mm_offset: Option<i64>,
    pub due_at_utc: Option<String>,
}

#[derive(Deserialize, AsChangeset)]
#[diesel(table_name = reminders)]
pub struct UpdateReminderInput {
    pub kind: Option<String>,
    pub mm_offset: Option<i64>,
    pub due_at_utc: Option<String>,
    pub scheduled_notification_id: Option<String>,
}
