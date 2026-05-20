use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::notification_snoozes;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = notification_snoozes)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct NotificationSnooze {
    pub reminder_id: String,
    pub fire_at_utc: String,
    pub created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = notification_snoozes)]
pub struct InsertNotificationSnooze {
    pub reminder_id: String,
    pub fire_at_utc: String,
    pub created_at: String,
}
