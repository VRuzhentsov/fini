use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::quests;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = quests)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Quest {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    /// "low" | "medium" | "high"
    pub energy: String,
    /// 1 = none, 2 = low, 3 = medium, 4 = urgent
    pub priority: i64,
    pub pinned: bool,
    pub due: Option<String>,
    pub due_time: Option<String>,
    /// JSON-encoded RepeatRule, or null
    pub repeat_rule: Option<String>,
    pub completed_at: Option<String>,
    pub set_main_at: Option<String>,
    pub reminder_triggered_at: Option<String>,
    pub order_rank: f64,
    pub created_at: String,
    pub updated_at: String,
    pub series_id: Option<String>,
    pub period_key: Option<String>,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = quests)]
pub struct CreateQuestInput {
    #[serde(default = "default_space_id")]
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(default = "default_energy")]
    pub energy: String,
    #[serde(default = "default_priority")]
    pub priority: i64,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
    pub order_rank: Option<f64>,
}

#[derive(Deserialize, AsChangeset)]
#[diesel(table_name = quests)]
pub struct UpdateQuestInput {
    pub space_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub energy: Option<String>,
    pub priority: Option<i64>,
    pub pinned: Option<bool>,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
    pub set_main_at: Option<String>,
    pub reminder_triggered_at: Option<String>,
    pub order_rank: Option<f64>,
}

pub fn default_priority() -> i64 {
    1
}

pub fn default_space_id() -> String {
    "1".to_string()
}

pub fn default_energy() -> String {
    "medium".to_string()
}

pub fn clamp_order_rank(value: f64) -> f64 {
    value.clamp(-100.0, 100.0)
}
