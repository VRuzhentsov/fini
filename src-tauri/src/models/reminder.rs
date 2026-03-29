use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::reminders;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = reminders)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Reminder {
    pub id: String,
    pub quest_id: String,
    pub kind: String,
    pub mm_offset: Option<i64>,
    pub due_at_utc: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = reminders)]
pub struct CreateReminderInput {
    pub quest_id: String,
    pub kind: String,
    pub mm_offset: Option<i64>,
    pub due_at_utc: Option<String>,
}
