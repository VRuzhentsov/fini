use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::quest_occurrences;

pub const OCCURRENCE_COMPLETED: &str = "completed";
pub const OCCURRENCE_ABANDONED: &str = "abandoned";

/// The stored row of one occurrence of a repeating quest (ADR-0012 D3): a
/// day that a person closed or changed. Every field after `closed_at`
/// overrides the quest's field for that day; `None` takes the quest's value
/// (D4). A checklist day's `description` is that day's own copy, with its
/// ticks (D5).
#[derive(Queryable, Selectable, Insertable, AsChangeset, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[diesel(table_name = quest_occurrences)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_null = true)]
pub struct QuestOccurrence {
    pub quest_id: String,
    /// The occurrence's date, `YYYY-MM-DD`.
    pub period_key: String,
    /// `None` while open, else `"completed"` or `"abandoned"`.
    pub status: Option<String>,
    pub closed_at: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub energy: Option<i64>,
    pub priority: Option<i64>,
    pub due_time: Option<String>,
    /// Device-local base of the day copy for the 3-way merge; never synced.
    #[serde(skip_serializing, default)]
    pub checklist_base: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl QuestOccurrence {
    /// A mark with nothing set yet: the occurrence `period_key` of `quest_id`.
    pub fn new(quest_id: &str, period_key: &str, now: &str) -> Self {
        Self {
            quest_id: quest_id.to_string(),
            period_key: period_key.to_string(),
            status: None,
            closed_at: None,
            title: None,
            description: None,
            energy: None,
            priority: None,
            due_time: None,
            checklist_base: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.status.is_some()
    }

    /// The sync entity id of the mark: one id for (quest, date) on every
    /// device.
    pub fn sync_id(&self) -> String {
        format!("{}:{}", self.quest_id, self.period_key)
    }
}
