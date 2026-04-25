use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use diesel::prelude::*;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

use crate::{
    models::{
        CreateQuestInput, CreateReminderInput, CreateSpaceInput, Quest, Reminder, Space,
        UpdateQuestInput,
    },
    schema::{quests, reminders, spaces},
    services::{
        db::open_db_at_path,
        quest::{append_focus_history, generate_next_occurrence, resolve_active_quest},
        reminder::{delete_reminder_db, upsert_reminder_db},
    },
};

// ── Server ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FiniServer {
    db: Arc<Mutex<SqliteConnection>>,
    tool_router: ToolRouter<FiniServer>,
}

impl FiniServer {
    pub fn new(db_path: &std::path::Path) -> Self {
        let conn = open_db_at_path(db_path);
        Self {
            db: Arc::new(Mutex::new(conn)),
            tool_router: Self::tool_router(),
        }
    }

    fn cli_structured(result: CallToolResult) -> serde_json::Value {
        result.structured_content.unwrap_or(serde_json::Value::Null)
    }

    pub async fn cli_list_quests(
        &self,
        space_id: Option<String>,
        status: Option<String>,
    ) -> Result<serde_json::Value, String> {
        self.list_quests(Parameters(ListQuestsParams { space_id, status }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_get_quest(&self, id: String) -> Result<serde_json::Value, String> {
        self.get_quest(Parameters(IdParam { id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_get_active_focus(&self) -> Result<serde_json::Value, String> {
        self.get_active_focus()
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_set_focus(
        &self,
        id: String,
        trigger: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let trigger_reminder_focus = match trigger.as_deref() {
            Some("reminder") => Some(true),
            _ => None,
        };

        self.update_quest(Parameters(UpdateQuestParams {
            id,
            title: None,
            description: None,
            status: None,
            space_id: None,
            pinned: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            set_focus: Some(true),
            trigger_reminder_focus,
        }))
        .await
        .map(Self::cli_structured)
        .map_err(|e| e.to_string())
    }

    pub async fn cli_create_quest(
        &self,
        title: String,
        space_id: Option<String>,
        description: Option<String>,
        due: Option<String>,
        due_time: Option<String>,
        repeat_rule: Option<String>,
    ) -> Result<serde_json::Value, String> {
        self.create_quest(Parameters(CreateQuestParams {
            title,
            space_id,
            description,
            due,
            due_time,
            repeat_rule,
        }))
        .await
        .map(Self::cli_structured)
        .map_err(|e| e.to_string())
    }

    pub async fn cli_update_quest(
        &self,
        params: UpdateQuestParams,
    ) -> Result<serde_json::Value, String> {
        self.update_quest(Parameters(params))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_complete_quest(&self, id: String) -> Result<serde_json::Value, String> {
        self.complete_quest(Parameters(IdParam { id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_abandon_quest(&self, id: String) -> Result<serde_json::Value, String> {
        self.abandon_quest(Parameters(IdParam { id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_delete_quest(&self, id: String) -> Result<serde_json::Value, String> {
        self.delete_quest(Parameters(IdParam { id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_list_history(&self) -> Result<serde_json::Value, String> {
        self.list_history()
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_list_spaces(&self) -> Result<serde_json::Value, String> {
        self.list_spaces()
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_create_space(&self, name: String) -> Result<serde_json::Value, String> {
        self.create_space(Parameters(CreateSpaceParams { name }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_update_space(
        &self,
        id: String,
        name: Option<String>,
    ) -> Result<serde_json::Value, String> {
        self.update_space(Parameters(UpdateSpaceParams { id, name }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_delete_space(&self, id: String) -> Result<serde_json::Value, String> {
        self.delete_space(Parameters(IdParam { id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_list_reminders(&self, quest_id: String) -> Result<serde_json::Value, String> {
        self.list_reminders(Parameters(QuestIdParam { quest_id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }

    pub async fn cli_create_reminder(
        &self,
        quest_id: String,
        kind: String,
        mm_offset: Option<i64>,
        due_at_utc: Option<String>,
    ) -> Result<serde_json::Value, String> {
        self.create_reminder(Parameters(CreateReminderParams {
            quest_id,
            kind,
            mm_offset,
            due_at_utc,
        }))
        .await
        .map(Self::cli_structured)
        .map_err(|e| e.to_string())
    }

    pub async fn cli_delete_reminder(&self, id: String) -> Result<serde_json::Value, String> {
        self.delete_reminder(Parameters(IdParam { id }))
            .await
            .map(Self::cli_structured)
            .map_err(|e| e.to_string())
    }
}

// ── Parameter types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct IdParam {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ListQuestsParams {
    #[schemars(description = "Filter by space id (optional)")]
    pub space_id: Option<String>,
    #[schemars(description = "Quest status: 'active' (default), 'completed', or 'abandoned'")]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CreateQuestParams {
    pub title: String,
    #[schemars(description = "Space id (optional, defaults to Personal)")]
    pub space_id: Option<String>,
    pub description: Option<String>,
    #[schemars(description = "ISO date string e.g. 2026-03-20")]
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateQuestParams {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[schemars(description = "Quest status: 'active', 'completed', or 'abandoned'")]
    pub status: Option<String>,
    pub space_id: Option<String>,
    pub pinned: Option<bool>,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
    pub order_rank: Option<f64>,
    #[schemars(description = "Set this quest as Focus now")]
    pub set_focus: Option<bool>,
    #[schemars(description = "Mark reminder-triggered focus now")]
    pub trigger_reminder_focus: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CreateSpaceParams {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateSpaceParams {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct QuestRecord {
    pub id: String,
    pub series_id: Option<String>,
    pub occurrence_id: Option<String>,
    pub period_key: Option<String>,
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub priority: i64,
    pub energy: String,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub due_at_utc: Option<String>,
    pub repeat_rule: Option<String>,
    pub order_rank: f64,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SpaceRecord {
    pub id: String,
    pub name: String,
    pub item_order: i64,
    pub created_at: String,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn db_err(e: diesel::result::Error) -> McpError {
    McpError::internal_error(
        "db_error",
        Some(serde_json::json!({ "error": e.to_string() })),
    )
}

fn parse_utc_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(ts) = DateTime::parse_from_rfc3339(value) {
        return Some(ts.with_timezone(&Utc));
    }
    if let Ok(ts) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Some(ts.and_utc());
    }
    if let Ok(ts) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Some(ts.and_utc());
    }
    None
}

fn due_at_utc_string(quest: &Quest) -> Option<String> {
    let due = quest.due.as_deref()?;
    let date = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
    let time = match quest.due_time.as_deref() {
        Some(value) => NaiveTime::parse_from_str(value, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S"))
            .ok()?,
        None => NaiveTime::from_hms_opt(23, 59, 59)?,
    };
    Some(
        NaiveDateTime::new(date, time)
            .and_utc()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
    )
}

fn period_key_for(quest: &Quest) -> Option<String> {
    if let Some(due) = quest.due.as_ref() {
        return Some(due.clone());
    }
    parse_utc_timestamp(&quest.created_at).map(|ts| ts.date_naive().format("%Y-%m-%d").to_string())
}

fn repeat_rule_active(rule: Option<&str>) -> bool {
    let Some(value) = rule else {
        return false;
    };
    let trimmed = value.trim();
    !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("none")
}

fn occurrence_fields(quest: &Quest) -> (Option<String>, Option<String>, Option<String>) {
    if !repeat_rule_active(quest.repeat_rule.as_deref()) {
        return (None, None, None);
    }
    let series_id = Some(quest.id.clone());
    let period_key = period_key_for(quest);
    let occurrence_id = match period_key.as_deref() {
        Some(key) => Some(format!("{}:{}", quest.id, key)),
        None => Some(quest.id.clone()),
    };
    (series_id, occurrence_id, period_key)
}

fn quest_to_record(quest: &Quest) -> QuestRecord {
    // Use real persisted series_id/period_key if present, fall back to computed
    let (series_id, occurrence_id, period_key) = if quest.series_id.is_some() {
        let sid = quest.series_id.clone();
        let pk = quest.period_key.clone();
        let oid = match (sid.as_deref(), pk.as_deref()) {
            (Some(s), Some(k)) => Some(format!("{}:{}", s, k)),
            (Some(s), None) => Some(s.to_string()),
            _ => None,
        };
        (sid, oid, pk)
    } else {
        occurrence_fields(quest)
    };
    QuestRecord {
        id: quest.id.clone(),
        series_id,
        occurrence_id,
        period_key,
        space_id: quest.space_id.clone(),
        title: quest.title.clone(),
        description: quest.description.clone(),
        status: quest.status.clone(),
        priority: quest.priority,
        energy: quest.energy.clone(),
        due: quest.due.clone(),
        due_time: quest.due_time.clone(),
        due_at_utc: due_at_utc_string(quest),
        repeat_rule: quest.repeat_rule.clone(),
        order_rank: quest.order_rank,
        completed_at: quest.completed_at.clone(),
        created_at: quest.created_at.clone(),
        updated_at: quest.updated_at.clone(),
    }
}

fn space_to_record(space: &Space) -> SpaceRecord {
    SpaceRecord {
        id: space.id.clone(),
        name: space.name.clone(),
        item_order: space.item_order,
        created_at: space.created_at.clone(),
    }
}

// ── Tools ──────────────────────────────────────────────────────────────────────

#[tool_router]
impl FiniServer {
    #[tool(
        description = "List quests. Defaults to active. Pass status='completed' or 'abandoned' for history."
    )]
    async fn list_quests(
        &self,
        Parameters(p): Parameters<ListQuestsParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let status = p.status.as_deref().unwrap_or("active");
        let mut query = quests::table
            .select(Quest::as_select())
            .filter(quests::status.eq(status))
            .order(quests::order_rank.asc())
            .into_boxed();
        if let Some(sid) = p.space_id {
            query = query.filter(quests::space_id.eq(sid));
        }
        let result = query.load(&mut *conn).map_err(db_err)?;
        let records: Vec<QuestRecord> = result.iter().map(quest_to_record).collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "quests": records,
        })))
    }

    #[tool(description = "Get a single quest by id.")]
    async fn get_quest(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let quest = quests::table
            .find(&p.id)
            .select(Quest::as_select())
            .first(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(quest_to_record(&quest)).unwrap(),
        ))
    }

    #[tool(description = "Get the current Focus quest from focus events + fallback rules.")]
    async fn get_active_focus(&self) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let quest = resolve_active_quest(&mut conn).map_err(db_err)?;
        match quest {
            Some(q) => Ok(CallToolResult::structured(
                serde_json::to_value(quest_to_record(&q)).unwrap(),
            )),
            None => Ok(CallToolResult::structured(serde_json::Value::Null)),
        }
    }

    #[tool(description = "Create a new quest.")]
    async fn create_quest(
        &self,
        Parameters(p): Parameters<CreateQuestParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let input = CreateQuestInput {
            space_id: p.space_id.unwrap_or_else(|| "1".to_string()),
            title: p.title,
            description: p.description,
            energy: "medium".to_string(),
            priority: 1,
            due: p.due,
            due_time: p.due_time,
            repeat_rule: p.repeat_rule,
            order_rank: None,
        };
        let quest: Quest = diesel::insert_into(quests::table)
            .values(&input)
            .returning(Quest::as_returning())
            .get_result(&mut *conn)
            .map_err(db_err)?;

        // Bridge: auto-create Reminder row when quest is created with a due date
        if quest.status == "active" && quest.due.is_some() {
            let _ = upsert_reminder_db(&mut conn, &quest);
        }

        Ok(CallToolResult::structured(
            serde_json::to_value(quest_to_record(&quest)).unwrap(),
        ))
    }

    #[tool(description = "Update a quest. Only provided fields are changed.")]
    async fn update_quest(
        &self,
        Parameters(p): Parameters<UpdateQuestParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let now = crate::services::db::utc_now();
        let input = UpdateQuestInput {
            space_id: p.space_id,
            title: p.title,
            description: p.description,
            status: p.status,
            energy: None,
            priority: None,
            pinned: p.pinned,
            due: p.due,
            due_time: p.due_time,
            repeat_rule: p.repeat_rule,
            order_rank: p.order_rank,
        };

        let status = input.status.clone();

        diesel::update(quests::table.find(&p.id))
            .set((&input, quests::updated_at.eq(&now)))
            .execute(&mut *conn)
            .map_err(db_err)?;

        let completed_at_update = match status.as_deref() {
            Some("completed") => Some(Some(now.clone())),
            Some("active") | Some("abandoned") => Some(None),
            _ => None,
        };

        if let Some(value) = completed_at_update {
            diesel::update(quests::table.find(&p.id))
                .set(quests::completed_at.eq(value))
                .execute(&mut *conn)
                .map_err(db_err)?;
        }

        let quest: Quest = quests::table
            .find(&p.id)
            .select(Quest::as_select())
            .first(&mut *conn)
            .map_err(db_err)?;

        if p.trigger_reminder_focus == Some(true) {
            append_focus_history(&mut *conn, &quest.id, &quest.space_id, "reminder").map_err(
                |e| {
                    McpError::internal_error(
                        "focus_history",
                        Some(serde_json::json!({ "error": e })),
                    )
                },
            )?;
        } else if p.set_focus == Some(true) || status.as_deref() == Some("active") {
            append_focus_history(&mut *conn, &quest.id, &quest.space_id, "manual").map_err(
                |e| {
                    McpError::internal_error(
                        "focus_history",
                        Some(serde_json::json!({ "error": e })),
                    )
                },
            )?;
        }

        // Bridge: keep Reminder row in sync with quest due/status
        if quest.status == "active" && quest.due.is_some() {
            let _ = upsert_reminder_db(&mut conn, &quest);
        } else {
            let _ = delete_reminder_db(&mut conn, &quest.id);
        }

        Ok(CallToolResult::structured(
            serde_json::to_value(quest_to_record(&quest)).unwrap(),
        ))
    }

    #[tool(description = "Mark a quest as completed.")]
    async fn complete_quest(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let now = crate::services::db::utc_now();
        diesel::update(quests::table.find(&p.id))
            .set((
                quests::status.eq("completed"),
                quests::completed_at.eq(&now),
                quests::updated_at.eq(&now),
            ))
            .execute(&mut *conn)
            .map_err(db_err)?;
        let quest: Quest = quests::table
            .find(&p.id)
            .select(Quest::as_select())
            .first(&mut *conn)
            .map_err(db_err)?;
        let _ = delete_reminder_db(&mut conn, &quest.id);
        if quest.repeat_rule.is_some() || quest.series_id.is_some() {
            if let Ok(Some(occ)) = generate_next_occurrence(&mut conn, &quest) {
                if occ.due.is_some() {
                    let _ = upsert_reminder_db(&mut conn, &occ);
                }
            }
        }
        Ok(CallToolResult::structured(
            serde_json::to_value(quest_to_record(&quest)).unwrap(),
        ))
    }

    #[tool(description = "Mark a quest as abandoned.")]
    async fn abandon_quest(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let now = crate::services::db::utc_now();
        diesel::update(quests::table.find(&p.id))
            .set((quests::status.eq("abandoned"), quests::updated_at.eq(&now)))
            .execute(&mut *conn)
            .map_err(db_err)?;
        let quest: Quest = quests::table
            .find(&p.id)
            .select(Quest::as_select())
            .first(&mut *conn)
            .map_err(db_err)?;
        let _ = delete_reminder_db(&mut conn, &quest.id);
        if quest.repeat_rule.is_some() || quest.series_id.is_some() {
            if let Ok(Some(occ)) = generate_next_occurrence(&mut conn, &quest) {
                if occ.due.is_some() {
                    let _ = upsert_reminder_db(&mut conn, &occ);
                }
            }
        }
        Ok(CallToolResult::structured(
            serde_json::to_value(quest_to_record(&quest)).unwrap(),
        ))
    }

    #[tool(description = "Permanently delete a quest.")]
    async fn delete_quest(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        diesel::delete(quests::table.find(&p.id))
            .execute(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(serde_json::json!({
            "deleted": true,
            "id": p.id,
        })))
    }

    #[tool(description = "List completed and abandoned quests (history).")]
    async fn list_history(&self) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let result: Vec<Quest> = quests::table
            .select(Quest::as_select())
            .filter(quests::status.ne("active"))
            .order(quests::updated_at.desc())
            .load(&mut *conn)
            .map_err(db_err)?;
        let records: Vec<QuestRecord> = result.iter().map(quest_to_record).collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "quests": records,
        })))
    }

    #[tool(description = "List all spaces.")]
    async fn list_spaces(&self) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let result: Vec<Space> = spaces::table
            .select(Space::as_select())
            .order(spaces::item_order.asc())
            .load(&mut *conn)
            .map_err(db_err)?;
        let records: Vec<SpaceRecord> = result.iter().map(space_to_record).collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "spaces": records,
        })))
    }

    #[tool(description = "Create a new space.")]
    async fn create_space(
        &self,
        Parameters(p): Parameters<CreateSpaceParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let item_order = spaces::table
            .count()
            .get_result::<i64>(&mut *conn)
            .unwrap_or(0);
        let input = CreateSpaceInput {
            name: p.name,
            item_order,
        };
        let space: Space = diesel::insert_into(spaces::table)
            .values(&input)
            .returning(Space::as_returning())
            .get_result(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(space_to_record(&space)).unwrap(),
        ))
    }

    #[tool(description = "Update a space name.")]
    async fn update_space(
        &self,
        Parameters(p): Parameters<UpdateSpaceParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        if let Some(name) = p.name {
            diesel::update(spaces::table.find(&p.id))
                .set(spaces::name.eq(&name))
                .execute(&mut *conn)
                .map_err(db_err)?;
        }
        let space: Space = spaces::table
            .find(&p.id)
            .select(Space::as_select())
            .first(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(space_to_record(&space)).unwrap(),
        ))
    }

    #[tool(description = "Delete a space.")]
    async fn delete_space(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        diesel::delete(spaces::table.find(&p.id))
            .execute(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(serde_json::json!({
            "deleted": true,
            "id": p.id,
        })))
    }

    // ── Reminder tools ───────────────────────────────────────────────────────

    #[tool(description = "List all reminders for a quest.")]
    async fn list_reminders(
        &self,
        Parameters(p): Parameters<QuestIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let items: Vec<Reminder> = reminders::table
            .filter(reminders::quest_id.eq(&p.quest_id))
            .select(Reminder::as_select())
            .load(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(items).unwrap(),
        ))
    }

    #[tool(
        description = "Create a reminder for a quest. Type 'relative' uses mm_offset (minutes before due). Type 'absolute' uses due_at_utc (ISO datetime)."
    )]
    async fn create_reminder(
        &self,
        Parameters(p): Parameters<CreateReminderParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let input = CreateReminderInput {
            quest_id: p.quest_id,
            kind: p.kind,
            mm_offset: p.mm_offset,
            due_at_utc: p.due_at_utc,
        };
        let reminder: Reminder = diesel::insert_into(reminders::table)
            .values(&input)
            .returning(Reminder::as_returning())
            .get_result(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(reminder).unwrap(),
        ))
    }

    #[tool(description = "Delete a reminder by ID.")]
    async fn delete_reminder(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        diesel::delete(reminders::table.find(&p.id))
            .execute(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::structured(serde_json::json!({
            "deleted": true,
            "id": p.id,
        })))
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
struct QuestIdParam {
    quest_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct CreateReminderParams {
    quest_id: String,
    #[serde(rename = "type")]
    kind: String,
    mm_offset: Option<i64>,
    due_at_utc: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("fini-mcp-{label}-{unique}.db"))
    }

    fn server_for(label: &str) -> (FiniServer, PathBuf) {
        let db_path = temp_db_path(label);
        let server = FiniServer::new(&db_path);
        (server, db_path)
    }

    fn json_id(value: &Value) -> String {
        value
            .get("id")
            .and_then(Value::as_str)
            .expect("id string")
            .to_string()
    }

    fn sample_quest() -> Quest {
        Quest {
            id: "quest-1".to_string(),
            space_id: "1".to_string(),
            title: "Quest".to_string(),
            description: None,
            status: "active".to_string(),
            energy: "medium".to_string(),
            priority: 1,
            pinned: false,
            due: None,
            due_time: None,
            repeat_rule: None,
            completed_at: None,
            order_rank: 0.0,
            created_at: "2026-03-27T10:30:00Z".to_string(),
            updated_at: "2026-03-27T10:30:00Z".to_string(),
            series_id: None,
            period_key: None,
        }
    }

    #[test]
    fn repeat_rule_active_handles_none_and_whitespace() {
        assert!(!repeat_rule_active(None));
        assert!(!repeat_rule_active(Some("")));
        assert!(!repeat_rule_active(Some("   ")));
        assert!(!repeat_rule_active(Some("none")));
        assert!(!repeat_rule_active(Some("NoNe")));
        assert!(repeat_rule_active(Some("daily")));
    }

    #[test]
    fn due_at_utc_string_supports_default_and_explicit_time() {
        let mut quest = sample_quest();
        quest.due = Some("2026-04-01".to_string());

        assert_eq!(
            due_at_utc_string(&quest),
            Some("2026-04-01T23:59:59Z".to_string())
        );

        quest.due_time = Some("09:15".to_string());
        assert_eq!(
            due_at_utc_string(&quest),
            Some("2026-04-01T09:15:00Z".to_string())
        );

        quest.due_time = Some("09:15:30".to_string());
        assert_eq!(
            due_at_utc_string(&quest),
            Some("2026-04-01T09:15:30Z".to_string())
        );
    }

    #[test]
    fn occurrence_fields_for_repeating_quest_use_due_or_created_date() {
        let mut quest = sample_quest();
        quest.repeat_rule = Some("daily".to_string());
        quest.due = Some("2026-04-02".to_string());

        let (series_id, occurrence_id, period_key) = occurrence_fields(&quest);
        assert_eq!(series_id, Some("quest-1".to_string()));
        assert_eq!(period_key, Some("2026-04-02".to_string()));
        assert_eq!(occurrence_id, Some("quest-1:2026-04-02".to_string()));

        quest.due = None;
        let (_, fallback_occurrence_id, fallback_period_key) = occurrence_fields(&quest);
        assert_eq!(fallback_period_key, Some("2026-03-27".to_string()));
        assert_eq!(
            fallback_occurrence_id,
            Some("quest-1:2026-03-27".to_string())
        );

        quest.created_at = "bad-created-at".to_string();
        let (_, no_period_occurrence_id, no_period_key) = occurrence_fields(&quest);
        assert_eq!(no_period_key, None);
        assert_eq!(no_period_occurrence_id, Some("quest-1".to_string()));
    }

    #[test]
    fn occurrence_fields_for_non_repeating_quest_are_empty() {
        let mut quest = sample_quest();
        quest.repeat_rule = None;

        let (series_id, occurrence_id, period_key) = occurrence_fields(&quest);
        assert_eq!(series_id, None);
        assert_eq!(occurrence_id, None);
        assert_eq!(period_key, None);
    }

    #[tokio::test]
    async fn list_quests_returns_occurrence_fields() {
        let (server, db_path) = server_for("list-quests-occurrence");
        {
            let mut conn = server.db.lock().expect("lock db");
            diesel::insert_into(quests::table)
                .values((
                    quests::space_id.eq("1"),
                    quests::title.eq("Daily quest"),
                    quests::status.eq("active"),
                    quests::due.eq("2026-03-20"),
                    quests::due_time.eq("09:00"),
                    quests::repeat_rule.eq("daily"),
                    quests::created_at.eq("2026-03-20T08:00:00Z"),
                    quests::updated_at.eq("2026-03-20T08:00:00Z"),
                ))
                .execute(&mut *conn)
                .expect("insert repeating quest");
        }

        let result = server
            .list_quests(Parameters(ListQuestsParams {
                space_id: None,
                status: None,
            }))
            .await
            .expect("list quests");

        let payload = result.structured_content.expect("structured content");
        let items = payload
            .get("quests")
            .and_then(Value::as_array)
            .expect("quests array payload");
        assert_eq!(items.len(), 1, "must return one quest");

        let item = &items[0];
        let id = json_id(item);
        assert_eq!(item["title"], "Daily quest");
        assert_eq!(item["series_id"].as_str().expect("series_id string"), id);
        assert_eq!(item["period_key"], "2026-03-20");
        assert_eq!(
            item["occurrence_id"]
                .as_str()
                .expect("occurrence_id string"),
            format!("{}:2026-03-20", id)
        );
        assert_eq!(item["due_at_utc"], "2026-03-20T09:00:00Z");
        assert_eq!(item["repeat_rule"], "daily");

        drop(server);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn quest_tools_return_structured_payloads() {
        let (server, db_path) = server_for("quest-tools-structured");

        let created = server
            .create_quest(Parameters(CreateQuestParams {
                title: "Create quest".to_string(),
                space_id: None,
                description: None,
                due: Some("2026-03-21".to_string()),
                due_time: Some("10:00".to_string()),
                repeat_rule: Some("daily".to_string()),
            }))
            .await
            .expect("create quest");

        let created_value = created
            .structured_content
            .expect("created structured content");
        let created_id = json_id(&created_value);

        let fetched = server
            .get_quest(Parameters(IdParam {
                id: created_id.clone(),
            }))
            .await
            .expect("get quest");
        let fetched_value = fetched.structured_content.expect("get structured content");
        assert_eq!(json_id(&fetched_value), created_id);

        let updated = server
            .update_quest(Parameters(UpdateQuestParams {
                id: created_id.clone(),
                title: None,
                description: Some("updated".to_string()),
                status: None,
                space_id: None,
                pinned: None,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                set_focus: Some(true),
                trigger_reminder_focus: None,
            }))
            .await
            .expect("update quest");
        let updated_value = updated
            .structured_content
            .expect("update structured content");
        assert_eq!(updated_value["description"], "updated");

        let completed = server
            .complete_quest(Parameters(IdParam {
                id: created_id.clone(),
            }))
            .await
            .expect("complete quest");
        let completed_value = completed
            .structured_content
            .expect("complete structured content");
        assert_eq!(completed_value["status"], "completed");

        let abandoned = server
            .create_quest(Parameters(CreateQuestParams {
                title: "Abandon quest".to_string(),
                space_id: None,
                description: None,
                due: None,
                due_time: None,
                repeat_rule: None,
            }))
            .await
            .expect("create quest to abandon");
        let abandoned_id = json_id(
            &abandoned
                .structured_content
                .expect("abandon created structured content"),
        );
        let abandoned_result = server
            .abandon_quest(Parameters(IdParam {
                id: abandoned_id.clone(),
            }))
            .await
            .expect("abandon quest");
        let abandoned_value = abandoned_result
            .structured_content
            .expect("abandon structured content");
        assert_eq!(abandoned_value["status"], "abandoned");

        let delete_target = server
            .create_quest(Parameters(CreateQuestParams {
                title: "Delete quest".to_string(),
                space_id: None,
                description: None,
                due: None,
                due_time: None,
                repeat_rule: None,
            }))
            .await
            .expect("create quest to delete");
        let delete_id = json_id(
            &delete_target
                .structured_content
                .expect("delete created structured content"),
        );
        let deleted = server
            .delete_quest(Parameters(IdParam { id: delete_id }))
            .await
            .expect("delete quest");
        let deleted_value = deleted
            .structured_content
            .expect("delete structured content");
        assert_eq!(deleted_value["deleted"], true);

        let history = server.list_history().await.expect("list history");
        let history_value = history
            .structured_content
            .expect("history structured content");
        let history_items = history_value
            .get("quests")
            .and_then(Value::as_array)
            .expect("history quests array");
        assert!(
            history_items
                .iter()
                .any(|item| item["status"] == "completed"),
            "history should include completed quest"
        );

        let active = server
            .create_quest(Parameters(CreateQuestParams {
                title: "Active quest".to_string(),
                space_id: None,
                description: None,
                due: None,
                due_time: None,
                repeat_rule: None,
            }))
            .await
            .expect("create active quest");
        let active_id = json_id(
            &active
                .structured_content
                .expect("active created structured content"),
        );
        let _ = server
            .update_quest(Parameters(UpdateQuestParams {
                id: active_id.clone(),
                title: None,
                description: None,
                status: None,
                space_id: None,
                pinned: None,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                set_focus: Some(true),
                trigger_reminder_focus: None,
            }))
            .await
            .expect("set main on active quest");

        let active_result = server.get_active_focus().await.expect("get active quest");
        let active_value = active_result
            .structured_content
            .expect("active structured content");
        assert_eq!(json_id(&active_value), active_id);

        drop(server);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn space_tools_return_structured_payloads() {
        let (server, db_path) = server_for("space-tools-structured");

        let created = server
            .create_space(Parameters(CreateSpaceParams {
                name: "Test Space".to_string(),
            }))
            .await
            .expect("create space");
        let created_value = created
            .structured_content
            .expect("space created structured content");
        let created_id = json_id(&created_value);

        let listed = server.list_spaces().await.expect("list spaces");
        let listed_value = listed
            .structured_content
            .expect("list spaces structured content");
        let listed_items = listed_value
            .get("spaces")
            .and_then(Value::as_array)
            .expect("spaces array");
        assert!(
            listed_items.iter().any(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .map(|id| id == created_id)
                    .unwrap_or(false)
            }),
            "list_spaces should include created space"
        );

        let updated = server
            .update_space(Parameters(UpdateSpaceParams {
                id: created_id.clone(),
                name: Some("Renamed Space".to_string()),
            }))
            .await
            .expect("update space");
        let updated_value = updated
            .structured_content
            .expect("space updated structured content");
        assert_eq!(updated_value["name"], "Renamed Space");

        let deleted = server
            .delete_space(Parameters(IdParam { id: created_id }))
            .await
            .expect("delete space");
        let deleted_value = deleted
            .structured_content
            .expect("space deleted structured content");
        assert_eq!(deleted_value["deleted"], true);

        drop(server);
        let _ = std::fs::remove_file(db_path);
    }
}

// ── ServerHandler ──────────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for FiniServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Quest management for Fini. Use tools to read and manage quests and spaces.",
        )
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

pub async fn run() -> Result<()> {
    let db_path = crate::services::db::db_default_path();
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let service = FiniServer::new(&db_path).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
