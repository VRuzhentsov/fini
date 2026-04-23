use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::focus_history;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = focus_history)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct FocusHistoryEntry {
    pub id: String,
    pub quest_id: String,
    pub space_id: String,
    pub trigger: String,
    pub created_at: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = focus_history)]
pub struct CreateFocusHistoryInput {
    pub quest_id: String,
    pub space_id: String,
    pub trigger: String,
}
