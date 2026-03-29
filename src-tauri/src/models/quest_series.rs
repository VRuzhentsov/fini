use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::quest_series;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = quest_series)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct QuestSeries {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    pub repeat_rule: String,
    pub priority: i64,
    pub energy: String,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = quest_series)]
pub struct CreateSeriesInput {
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    pub repeat_rule: String,
    pub priority: i64,
    pub energy: String,
}
