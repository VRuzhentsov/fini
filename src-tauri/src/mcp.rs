use std::sync::{Arc, Mutex};

use anyhow::Result;
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
    resolve_active_quest,
    schema::{quests, spaces},
    CreateQuestInput, CreateSpaceInput, Quest, Space, UpdateQuestInput,
};

// ── Server ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FiniServer {
    db: Arc<Mutex<SqliteConnection>>,
    tool_router: ToolRouter<FiniServer>,
}

impl FiniServer {
    pub fn new(db_path: &std::path::Path) -> Self {
        let conn = crate::open_db_at_path(db_path);
        Self {
            db: Arc::new(Mutex::new(conn)),
            tool_router: Self::tool_router(),
        }
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
    #[schemars(description = "Set this quest as Main now")]
    pub set_main: Option<bool>,
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

// ── Helpers ────────────────────────────────────────────────────────────────────

fn db_err(e: diesel::result::Error) -> McpError {
    McpError::internal_error(
        "db_error",
        Some(serde_json::json!({ "error": e.to_string() })),
    )
}

fn quests_to_text(quests: &[Quest]) -> String {
    if quests.is_empty() {
        return "No quests found.".to_string();
    }
    quests
        .iter()
        .map(|q| format!("[{}] {} ({})", q.id, q.title, q.status))
        .collect::<Vec<_>>()
        .join("\n")
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
        Ok(CallToolResult::success(vec![Content::text(
            quests_to_text(&result),
        )]))
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
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&quest).unwrap(),
        )]))
    }

    #[tool(description = "Get the current Main quest from focus events + fallback rules.")]
    async fn get_active_quest(&self) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let quest = resolve_active_quest(&mut conn).map_err(db_err)?;
        match quest {
            Some(q) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&q).unwrap(),
            )])),
            None => Ok(CallToolResult::success(vec![Content::text(
                "No active quest.".to_string(),
            )])),
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
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&quest).unwrap(),
        )]))
    }

    #[tool(description = "Update a quest. Only provided fields are changed.")]
    async fn update_quest(
        &self,
        Parameters(p): Parameters<UpdateQuestParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let now = crate::utc_now();
        let mut input = UpdateQuestInput {
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
            set_main_at: None,
            reminder_triggered_at: None,
        };

        if p.set_main == Some(true) || input.status.as_deref() == Some("active") {
            input.set_main_at = Some(now.clone());
        }
        if p.trigger_reminder_focus == Some(true) {
            input.reminder_triggered_at = Some(now.clone());
        }

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
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&quest).unwrap(),
        )]))
    }

    #[tool(description = "Mark a quest as completed.")]
    async fn complete_quest(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let now = crate::utc_now();
        diesel::update(quests::table.find(&p.id))
            .set((
                quests::status.eq("completed"),
                quests::completed_at.eq(&now),
                quests::updated_at.eq(&now),
            ))
            .execute(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::success(vec![Content::text(
            "Quest completed.".to_string(),
        )]))
    }

    #[tool(description = "Mark a quest as abandoned.")]
    async fn abandon_quest(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let now = crate::utc_now();
        diesel::update(quests::table.find(&p.id))
            .set((quests::status.eq("abandoned"), quests::updated_at.eq(&now)))
            .execute(&mut *conn)
            .map_err(db_err)?;
        Ok(CallToolResult::success(vec![Content::text(
            "Quest abandoned.".to_string(),
        )]))
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
        Ok(CallToolResult::success(vec![Content::text(
            "Quest deleted.".to_string(),
        )]))
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
        Ok(CallToolResult::success(vec![Content::text(
            quests_to_text(&result),
        )]))
    }

    #[tool(description = "List all spaces.")]
    async fn list_spaces(&self) -> Result<CallToolResult, McpError> {
        let mut conn = self.db.lock().unwrap();
        let result: Vec<Space> = spaces::table
            .select(Space::as_select())
            .order(spaces::item_order.asc())
            .load(&mut *conn)
            .map_err(db_err)?;
        let text = result
            .iter()
            .map(|s| format!("[{}] {}", s.id, s.name))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(CallToolResult::success(vec![Content::text(text)]))
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
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&space).unwrap(),
        )]))
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
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&space).unwrap(),
        )]))
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
        Ok(CallToolResult::success(vec![Content::text(
            "Space deleted.".to_string(),
        )]))
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
    let db_path = crate::db_default_path();
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let service = FiniServer::new(&db_path).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
