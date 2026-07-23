use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::checklist_activity;

/// Audit trail for a quest's checklist. Written on item add/remove/text-edit, quest completion
/// (`completed_snapshot`), and any edit made after completion (`post_completion_edit`) — not on
/// every individual check/uncheck. See issue #128's "auditable" requirement for completed
/// history.
#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = checklist_activity)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ChecklistActivity {
    pub id: String,
    pub quest_id: String,
    /// "added" | "removed" | "edited" | "completed_snapshot" | "post_completion_edit"
    pub kind: String,
    pub detail: String,
    pub created_at: String,
    pub origin_device_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = checklist_activity)]
pub struct CreateChecklistActivityInput {
    pub quest_id: String,
    pub kind: String,
    pub detail: String,
    pub created_at: String,
    pub origin_device_id: Option<String>,
}
