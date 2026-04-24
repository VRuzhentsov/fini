use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{reminders, series_reminder_templates};

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
    pub scheduled_notification_id: Option<String>,
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

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = series_reminder_templates)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct SeriesReminderTemplate {
    pub id: String,
    pub series_id: String,
    pub kind: String,
    pub mm_offset: Option<i64>,
    pub created_at: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = series_reminder_templates)]
pub struct CreateSeriesReminderTemplateInput {
    pub series_id: String,
    pub kind: String,
    pub mm_offset: Option<i64>,
}
