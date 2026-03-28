mod services;
pub mod mcp;
mod schema;
// mod voice;       // postponed
// mod model_download; // postponed

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use services::device_sync::{
    device_discovery_snapshot, device_enter_add_mode, device_get_identity, device_leave_add_mode,
    device_presence_snapshot,
    device_pair_accept_request, device_pair_acknowledge_request, device_pair_complete_request,
    device_pair_incoming_requests, device_pair_outgoing_completions, device_pair_outgoing_updates,
    device_send_pair_request, device_sync_debug_status, DeviceSyncState,
};
use schema::{quest_series, quests, reminders, spaces};

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Manager, State};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub struct DbState(pub Mutex<SqliteConnection>);

pub fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ── DB models (Queryable) ─────────────────────────────────────────────────────

#[derive(Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = spaces)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Space {
    pub id: String,
    pub name: String,
    pub item_order: i64,
    pub created_at: String,
}

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

// ── Command input types ───────────────────────────────────────────────────────

#[derive(Deserialize, Insertable)]
#[diesel(table_name = spaces)]
pub struct CreateSpaceInput {
    pub name: String,
    pub item_order: i64,
}

#[derive(Deserialize, AsChangeset)]
#[diesel(table_name = spaces)]
pub struct UpdateSpaceInput {
    pub name: Option<String>,
    pub item_order: Option<i64>,
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

fn default_priority() -> i64 {
    1
}
fn default_space_id() -> String {
    "1".to_string()
}
fn default_energy() -> String {
    "medium".to_string()
}

fn clamp_order_rank(value: f64) -> f64 {
    value.clamp(-100.0, 100.0)
}

// ── Repeat rule logic ────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, Clone, Debug)]
struct RepeatRule {
    preset: Option<String>,
    interval: Option<i64>,
    unit: Option<String>,
    days_of_week: Option<Vec<String>>,
    end: Option<String>,
    end_date: Option<String>,
    end_after: Option<i64>,
}

fn compute_next_due(current_due: &NaiveDate, rule: &RepeatRule) -> Option<NaiveDate> {
    use chrono::{Datelike, Months, Weekday};

    match rule.preset.as_deref() {
        Some("daily") => Some(*current_due + chrono::Duration::days(1)),
        Some("weekdays") => {
            let mut next = *current_due + chrono::Duration::days(1);
            while next.weekday() == Weekday::Sat || next.weekday() == Weekday::Sun {
                next += chrono::Duration::days(1);
            }
            Some(next)
        }
        Some("weekends") => {
            let mut next = *current_due + chrono::Duration::days(1);
            while next.weekday() != Weekday::Sat && next.weekday() != Weekday::Sun {
                next += chrono::Duration::days(1);
            }
            Some(next)
        }
        Some("weekly") => Some(*current_due + chrono::Duration::weeks(1)),
        Some("monthly") => current_due.checked_add_months(Months::new(1)),
        Some("yearly") => current_due.checked_add_months(Months::new(12)),
        Some("custom") => {
            let interval = rule.interval.unwrap_or(1).max(1);
            match rule.unit.as_deref() {
                Some("day") => Some(*current_due + chrono::Duration::days(interval)),
                Some("week") => Some(*current_due + chrono::Duration::weeks(interval)),
                Some("month") => current_due.checked_add_months(Months::new(interval as u32)),
                Some("year") => current_due.checked_add_months(Months::new(interval as u32 * 12)),
                _ => None,
            }
        }
        Some("none") | None => None,
        _ => None,
    }
}

fn is_series_end_reached(rule: &RepeatRule, next_due: &NaiveDate, completed_count: i64) -> bool {
    match rule.end.as_deref() {
        Some("on_date") => {
            if let Some(ref end_date_str) = rule.end_date {
                if let Ok(end_date) = NaiveDate::parse_from_str(end_date_str, "%Y-%m-%d") {
                    return *next_due > end_date;
                }
            }
            false
        }
        Some("after_n") => {
            if let Some(max) = rule.end_after {
                return completed_count >= max;
            }
            false
        }
        _ => false,
    }
}

fn generate_next_occurrence(
    conn: &mut SqliteConnection,
    quest: &Quest,
) -> Result<Option<Quest>, diesel::result::Error> {
    let series_id = match quest.series_id.as_ref() {
        Some(id) => id.clone(),
        None => return Ok(None),
    };

    let series: QuestSeries = match quest_series::table
        .find(&series_id)
        .select(QuestSeries::as_select())
        .first(conn)
    {
        Ok(s) => s,
        Err(diesel::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };

    if !series.active {
        return Ok(None);
    }

    let rule: RepeatRule = match serde_json::from_str(&series.repeat_rule) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let current_due = quest
        .due
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .unwrap_or_else(|| Utc::now().date_naive());

    let next_due = match compute_next_due(&current_due, &rule) {
        Some(d) => d,
        None => return Ok(None),
    };

    // Check end conditions
    let completed_count: i64 = quests::table
        .filter(quests::series_id.eq(&series_id))
        .filter(quests::status.ne("active"))
        .count()
        .get_result(conn)?;

    if is_series_end_reached(&rule, &next_due, completed_count) {
        diesel::update(quest_series::table.find(&series_id))
            .set(quest_series::active.eq(false))
            .execute(conn)?;
        return Ok(None);
    }

    let period_key = next_due.format("%Y-%m-%d").to_string();

    // Check if this occurrence already exists (deterministic dedup)
    let existing = quests::table
        .filter(quests::series_id.eq(&series_id))
        .filter(quests::period_key.eq(&period_key))
        .count()
        .get_result::<i64>(conn)?;

    if existing > 0 {
        return Ok(None);
    }

    let now = utc_now();
    let max_rank = quests::table
        .select(diesel::dsl::max(quests::order_rank))
        .first::<Option<f64>>(conn)?
        .unwrap_or(0.0);

    diesel::insert_into(quests::table)
        .values((
            quests::space_id.eq(&series.space_id),
            quests::title.eq(&series.title),
            quests::description.eq(&series.description),
            quests::status.eq("active"),
            quests::energy.eq(&series.energy),
            quests::priority.eq(series.priority),
            quests::due.eq(&period_key),
            quests::due_time.eq(quest.due_time.as_deref()),
            quests::repeat_rule.eq(&series.repeat_rule),
            quests::order_rank.eq(max_rank + 1.0),
            quests::series_id.eq(&series_id),
            quests::period_key.eq(&period_key),
            quests::created_at.eq(&now),
            quests::updated_at.eq(&now),
        ))
        .execute(conn)?;

    quests::table
        .filter(quests::series_id.eq(&series_id))
        .filter(quests::period_key.eq(&period_key))
        .select(Quest::as_select())
        .first(conn)
        .optional()
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

fn parse_due_deadline_utc(quest: &Quest) -> Option<DateTime<Utc>> {
    let due = quest.due.as_deref()?;
    let date = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
    let time = match quest.due_time.as_deref() {
        Some(value) => NaiveTime::parse_from_str(value, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S"))
            .ok()?,
        None => NaiveTime::from_hms_opt(23, 59, 59)?,
    };
    Some(NaiveDateTime::new(date, time).and_utc())
}

fn is_overdue(quest: &Quest, now: &DateTime<Utc>) -> bool {
    parse_due_deadline_utc(quest)
        .map(|deadline| deadline < *now)
        .unwrap_or(false)
}

fn latest_focus_timestamp(quest: &Quest) -> Option<DateTime<Utc>> {
    [
        quest.set_main_at.as_deref().and_then(parse_utc_timestamp),
        quest
            .reminder_triggered_at
            .as_deref()
            .and_then(parse_utc_timestamp),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn resolve_active_from_loaded(loaded: &[Quest]) -> Option<Quest> {
    let active: Vec<&Quest> = loaded.iter().filter(|q| q.status == "active").collect();
    if active.is_empty() {
        return None;
    }

    let mut focus_candidates: Vec<(&Quest, DateTime<Utc>)> = active
        .iter()
        .filter_map(|quest| latest_focus_timestamp(quest).map(|ts| (*quest, ts)))
        .collect();

    if !focus_candidates.is_empty() {
        focus_candidates.sort_by(|(qa, ta), (qb, tb)| tb.cmp(ta).then_with(|| qa.id.cmp(&qb.id)));
        return Some(focus_candidates[0].0.clone());
    }

    let now = Utc::now();
    let mut fallback = active;
    fallback.sort_by(|a, b| {
        let a_overdue = is_overdue(a, &now);
        let b_overdue = is_overdue(b, &now);

        if a_overdue != b_overdue {
            return b_overdue.cmp(&a_overdue);
        }

        if (a.order_rank - b.order_rank).abs() > f64::EPSILON {
            return a
                .order_rank
                .partial_cmp(&b.order_rank)
                .unwrap_or(std::cmp::Ordering::Equal);
        }

        if a.priority != b.priority {
            return b.priority.cmp(&a.priority);
        }

        match (
            parse_utc_timestamp(&a.created_at),
            parse_utc_timestamp(&b.created_at),
        ) {
            (Some(a_created), Some(b_created)) if a_created != b_created => {
                a_created.cmp(&b_created)
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.created_at.cmp(&b.created_at),
        }
        .then_with(|| a.id.cmp(&b.id))
    });

    Some((*fallback[0]).clone())
}

pub fn resolve_active_quest(
    conn: &mut SqliteConnection,
) -> Result<Option<Quest>, diesel::result::Error> {
    let loaded: Vec<Quest> = quests::table.select(Quest::as_select()).load(conn)?;
    Ok(resolve_active_from_loaded(&loaded))
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

// ── DB setup ──────────────────────────────────────────────────────────────────

pub fn db_default_path() -> std::path::PathBuf {
    dirs::data_dir()
        .expect("failed to get data dir")
        .join("com.fini.app")
        .join("fini.db")
}

pub fn open_db_at_path(path: &std::path::Path) -> SqliteConnection {
    let mut conn =
        SqliteConnection::establish(path.to_str().unwrap()).expect("failed to open database");
    diesel::sql_query("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .execute(&mut conn)
        .expect("failed to set PRAGMAs");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("failed to run migrations");
    conn
}

fn open_db(app: &tauri::AppHandle) -> SqliteConnection {
    let data_dir = app
        .path()
        .app_data_dir()
        .expect("failed to resolve app data dir");
    std::fs::create_dir_all(&data_dir).expect("failed to create app data dir");
    let db_path = data_dir.join("fini.db");
    open_db_at_path(&db_path)
}

// ── Space commands ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_spaces(state: State<DbState>) -> Result<Vec<Space>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    spaces::table
        .select(Space::as_select())
        .order(spaces::item_order.asc())
        .load(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_space(state: State<DbState>, input: CreateSpaceInput) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::insert_into(spaces::table)
        .values(&input)
        .returning(Space::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_space(
    state: State<DbState>,
    id: String,
    input: UpdateSpaceInput,
) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::update(spaces::table.find(&id))
        .set(&input)
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    spaces::table
        .find(&id)
        .select(Space::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_space(state: State<DbState>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(spaces::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Quest commands ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_quests(state: State<DbState>) -> Result<Vec<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let mut loaded: Vec<Quest> = quests::table
        .select(Quest::as_select())
        .load(&mut *conn)
        .map_err(|e| e.to_string())?;

    let now = Utc::now();
    loaded.sort_by(|a, b| {
        let a_active = a.status == "active";
        let b_active = b.status == "active";
        if a_active != b_active {
            return b_active.cmp(&a_active);
        }

        if a_active {
            let a_overdue = is_overdue(a, &now);
            let b_overdue = is_overdue(b, &now);
            if a_overdue != b_overdue {
                return b_overdue.cmp(&a_overdue);
            }

            if (a.order_rank - b.order_rank).abs() > f64::EPSILON {
                return a
                    .order_rank
                    .partial_cmp(&b.order_rank)
                    .unwrap_or(std::cmp::Ordering::Equal);
            }

            if a.priority != b.priority {
                return b.priority.cmp(&a.priority);
            }
            return match (
                parse_utc_timestamp(&a.created_at),
                parse_utc_timestamp(&b.created_at),
            ) {
                (Some(a_created), Some(b_created)) if a_created != b_created => {
                    a_created.cmp(&b_created)
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => a.created_at.cmp(&b.created_at),
            }
            .then_with(|| a.id.cmp(&b.id));
        }

        match (
            parse_utc_timestamp(&a.updated_at),
            parse_utc_timestamp(&b.updated_at),
        ) {
            (Some(a_updated), Some(b_updated)) if a_updated != b_updated => {
                b_updated.cmp(&a_updated)
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => b.updated_at.cmp(&a.updated_at),
        }
        .then_with(|| a.id.cmp(&b.id))
    });

    Ok(loaded)
}

#[tauri::command]
fn create_quest(state: State<DbState>, input: CreateQuestInput) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();

    let max_rank = quests::table
        .select(diesel::dsl::max(quests::order_rank))
        .first::<Option<f64>>(&mut *conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0.0);

    let has_repeat = input
        .repeat_rule
        .as_deref()
        .map(|r| !r.is_empty())
        .unwrap_or(false);

    if has_repeat {
        let repeat_rule_str = input.repeat_rule.clone().unwrap();
        let series_input = CreateSeriesInput {
            space_id: input.space_id.clone(),
            title: input.title.clone(),
            description: input.description.clone(),
            repeat_rule: repeat_rule_str,
            priority: input.priority,
            energy: input.energy.clone(),
        };

        let series = diesel::insert_into(quest_series::table)
            .values(&series_input)
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut *conn)
            .map_err(|e| e.to_string())?;

        let period_key = input
            .due
            .as_deref()
            .unwrap_or(&Utc::now().format("%Y-%m-%d").to_string())
            .to_string();

        let now = utc_now();
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq(&input.space_id),
                quests::title.eq(&input.title),
                quests::description.eq(&input.description),
                quests::status.eq("active"),
                quests::energy.eq(&input.energy),
                quests::priority.eq(input.priority),
                quests::due.eq(&input.due),
                quests::due_time.eq(&input.due_time),
                quests::repeat_rule.eq(&input.repeat_rule),
                quests::order_rank.eq(clamp_order_rank(
                    input.order_rank.unwrap_or(max_rank + 1.0),
                )),
                quests::series_id.eq(&series.id),
                quests::period_key.eq(&period_key),
                quests::created_at.eq(&now),
                quests::updated_at.eq(&now),
            ))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;

        return quests::table
            .filter(quests::series_id.eq(&series.id))
            .filter(quests::period_key.eq(&period_key))
            .select(Quest::as_select())
            .first(&mut *conn)
            .map_err(|e| e.to_string());
    }

    let payload = CreateQuestInput {
        order_rank: Some(clamp_order_rank(input.order_rank.unwrap_or(max_rank + 1.0))),
        ..input
    };

    diesel::insert_into(quests::table)
        .values(&payload)
        .returning(Quest::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_active_quest(state: State<DbState>) -> Result<Option<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    resolve_active_quest(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_main_quest(state: State<DbState>, id: String) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let now = utc_now();

    let updated = diesel::update(quests::table.find(&id).filter(quests::status.eq("active")))
        .set((quests::set_main_at.eq(&now), quests::updated_at.eq(&now)))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;

    if updated == 0 {
        return Err("cannot set Main on non-active quest".to_string());
    }

    quests::table
        .find(&id)
        .select(Quest::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_quest(
    state: State<DbState>,
    id: String,
    input: UpdateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let now = utc_now();

    let mut patch = input;
    let status = patch.status.clone();

    if let Some(rank) = patch.order_rank {
        patch.order_rank = Some(clamp_order_rank(rank));
    }

    if status.as_deref() == Some("active") && patch.set_main_at.is_none() {
        patch.set_main_at = Some(now.clone());
    }

    diesel::update(quests::table.find(&id))
        .set((&patch, quests::updated_at.eq(&now)))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;

    let completed_at_update = match status.as_deref() {
        Some("completed") => Some(Some(now)),
        Some("active") | Some("abandoned") => Some(None),
        _ => None,
    };

    if let Some(val) = completed_at_update {
        diesel::update(quests::table.find(&id))
            .set(quests::completed_at.eq(val))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    }

    let updated_quest: Quest = quests::table
        .find(&id)
        .select(Quest::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())?;

    // Auto-generate next occurrence when a series quest is completed or abandoned
    if matches!(status.as_deref(), Some("completed") | Some("abandoned")) {
        if updated_quest.series_id.is_some() {
            let _ = generate_next_occurrence(&mut conn, &updated_quest);
        }
    }

    Ok(updated_quest)
}

#[tauri::command]
fn delete_quest(state: State<DbState>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(quests::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Reminder commands ─────────────────────────────────────────────────────────

#[tauri::command]
fn get_reminders(state: State<DbState>, quest_id: String) -> Result<Vec<Reminder>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    reminders::table
        .filter(reminders::quest_id.eq(&quest_id))
        .select(Reminder::as_select())
        .load(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_reminder(state: State<DbState>, input: CreateReminderInput) -> Result<Reminder, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::insert_into(reminders::table)
        .values(&input)
        .returning(Reminder::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_reminder(state: State<DbState>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(reminders::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── MCP entry point ───────────────────────────────────────────────────────────

pub fn run_mcp() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(mcp::run())
        .unwrap();
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let conn = open_db(&app.handle());
            app.manage(DbState(Mutex::new(conn)));

            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            app.manage(DeviceSyncState::new(&data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_spaces,
            create_space,
            update_space,
            delete_space,
            get_quests,
            get_active_quest,
            create_quest,
            set_main_quest,
            update_quest,
            delete_quest,
            get_reminders,
            create_reminder,
            delete_reminder,
            device_get_identity,
            device_enter_add_mode,
            device_leave_add_mode,
            device_discovery_snapshot,
            device_presence_snapshot,
            device_send_pair_request,
            device_pair_incoming_requests,
            device_pair_outgoing_updates,
            device_pair_outgoing_completions,
            device_pair_accept_request,
            device_pair_complete_request,
            device_pair_acknowledge_request,
            device_sync_debug_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("fini-{label}-{unique}.db"))
    }

    fn is_uuid_like(value: &str) -> bool {
        value.len() == 36
            && value.as_bytes()[8] == b'-'
            && value.as_bytes()[13] == b'-'
            && value.as_bytes()[18] == b'-'
            && value.as_bytes()[23] == b'-'
    }

    #[test]
    fn order_rank_is_clamped_to_signed_100() {
        assert_eq!(clamp_order_rank(150.0), 100.0);
        assert_eq!(clamp_order_rank(-150.0), -100.0);
        assert_eq!(clamp_order_rank(42.5), 42.5);
    }

    fn execute_sql_script(conn: &mut SqliteConnection, script: &str) {
        for statement in script.split(';') {
            let sql = statement.trim();
            if sql.is_empty() {
                continue;
            }
            diesel::sql_query(sql)
                .execute(conn)
                .expect("execute SQL statement from script");
        }
    }

    fn insert_active_quest(
        conn: &mut SqliteConnection,
        title: &str,
        priority: i64,
        created_at: &str,
        due: Option<&str>,
        due_time: Option<&str>,
    ) -> String {
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq(title),
                quests::status.eq("active"),
                quests::priority.eq(priority),
                quests::due.eq(due.map(str::to_string)),
                quests::due_time.eq(due_time.map(str::to_string)),
                quests::created_at.eq(created_at),
                quests::updated_at.eq(created_at),
            ))
            .execute(conn)
            .expect("insert test active quest");

        quests::table
            .filter(quests::title.eq(title))
            .select(quests::id)
            .first(conn)
            .expect("load inserted quest id")
    }

    #[test]
    fn quests_space_id_rejects_null() {
        let db_path = temp_db_path("quests-space-id-rejects-null");
        let mut conn = open_db_at_path(&db_path);

        let insert_result = diesel::sql_query(
            "INSERT INTO quests (space_id, title) VALUES (NULL, 'nullable-space-id-should-fail')",
        )
        .execute(&mut conn);

        assert!(
            insert_result.is_err(),
            "quests.space_id must be non-null and reject NULL inserts"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn quests_space_id_defaults_to_personal() {
        let db_path = temp_db_path("quests-space-id-defaults-to-personal");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values(quests::title.eq("defaults-to-personal"))
            .execute(&mut conn)
            .expect("insert without space_id should succeed with default");

        let rows: Vec<String> = quests::table
            .filter(quests::title.eq("defaults-to-personal"))
            .select(quests::space_id)
            .load(&mut conn)
            .expect("query inserted quest");

        assert_eq!(rows.len(), 1, "must insert exactly one quest row");
        assert_eq!(rows[0], "1", "default quest space_id must be 1");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn deleting_custom_space_reassigns_quests_to_personal() {
        let db_path = temp_db_path("deleting-custom-space-reassigns-quests-to-personal");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(spaces::table)
            .values((
                spaces::id.eq("test-custom-space"),
                spaces::name.eq("Custom"),
                spaces::item_order.eq(99_i64),
            ))
            .execute(&mut conn)
            .expect("insert custom space");

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("test-custom-space"),
                quests::title.eq("reassign-on-delete"),
            ))
            .execute(&mut conn)
            .expect("insert quest in custom space");

        diesel::delete(spaces::table.find("test-custom-space"))
            .execute(&mut conn)
            .expect("delete custom space");

        let rows: Vec<String> = quests::table
            .filter(quests::title.eq("reassign-on-delete"))
            .select(quests::space_id)
            .load(&mut conn)
            .expect("query reassigned quest");

        assert_eq!(rows.len(), 1, "quest must still exist");
        assert_eq!(
            rows[0], "1",
            "deleting custom space must reassign quest space_id to 1"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn built_in_space_ids_exist_after_migration() {
        let db_path = temp_db_path("built-in-space-ids-exist-after-migration");
        let mut conn = open_db_at_path(&db_path);

        let ids: Vec<String> = spaces::table
            .select(spaces::id)
            .load(&mut conn)
            .expect("load spaces ids");

        assert!(
            ids.iter().any(|id| id == "1"),
            "Personal space id=1 must exist"
        );
        assert!(
            ids.iter().any(|id| id == "2"),
            "Family space id=2 must exist"
        );
        assert!(ids.iter().any(|id| id == "3"), "Work space id=3 must exist");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn new_quest_ids_are_uuid_like_strings() {
        let db_path = temp_db_path("new-quest-ids-are-uuid-like-strings");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values(quests::title.eq("uuid-shape-check"))
            .execute(&mut conn)
            .expect("insert quest to check id shape");

        let ids: Vec<String> = quests::table
            .filter(quests::title.eq("uuid-shape-check"))
            .select(quests::id)
            .load(&mut conn)
            .expect("load created quest id");

        assert_eq!(ids.len(), 1, "must have exactly one created quest");
        let id = &ids[0];
        assert!(is_uuid_like(id), "quest id must look like UUID");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn manual_set_main_overrides_fallback_selection() {
        let db_path = temp_db_path("manual-set-main-overrides-fallback-selection");
        let mut conn = open_db_at_path(&db_path);

        let low_priority_id = insert_active_quest(
            &mut conn,
            "manual-main-low",
            1,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let high_priority_id = insert_active_quest(
            &mut conn,
            "manual-main-high",
            4,
            "2026-03-02T10:00:00Z",
            None,
            None,
        );

        let before = resolve_active_quest(&mut conn)
            .expect("resolve before set-main")
            .expect("must return active quest");
        assert_eq!(
            before.id, high_priority_id,
            "without overrides, fallback should pick higher priority"
        );

        diesel::update(quests::table.find(&low_priority_id))
            .set(quests::set_main_at.eq("2026-03-03T12:00:00Z"))
            .execute(&mut conn)
            .expect("set manual main timestamp");

        let after = resolve_active_quest(&mut conn)
            .expect("resolve after set-main")
            .expect("must return active quest");
        assert_eq!(
            after.id, low_priority_id,
            "manual set-main timestamp must override fallback ordering"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn reminder_preemption_unwinds_after_resolution() {
        let db_path = temp_db_path("reminder-preemption-unwinds-after-resolution");
        let mut conn = open_db_at_path(&db_path);

        let manual_id = insert_active_quest(
            &mut conn,
            "manual-main",
            2,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let reminder_id = insert_active_quest(
            &mut conn,
            "reminder-main",
            2,
            "2026-03-01T11:00:00Z",
            None,
            None,
        );

        diesel::update(quests::table.find(&manual_id))
            .set(quests::set_main_at.eq("2026-03-05T09:00:00Z"))
            .execute(&mut conn)
            .expect("set manual override");

        let before_preempt = resolve_active_quest(&mut conn)
            .expect("resolve before reminder")
            .expect("must return active quest");
        assert_eq!(
            before_preempt.id, manual_id,
            "manual override should be active before reminder"
        );

        diesel::update(quests::table.find(&reminder_id))
            .set(quests::reminder_triggered_at.eq("2026-03-05T09:30:00Z"))
            .execute(&mut conn)
            .expect("set reminder override");

        let preempted = resolve_active_quest(&mut conn)
            .expect("resolve during reminder")
            .expect("must return active quest");
        assert_eq!(
            preempted.id, reminder_id,
            "latest reminder timestamp should preempt manual Main"
        );

        diesel::update(quests::table.find(&reminder_id))
            .set(quests::status.eq("completed"))
            .execute(&mut conn)
            .expect("resolve reminder quest by completion");

        let unwound = resolve_active_quest(&mut conn)
            .expect("resolve after reminder completion")
            .expect("must return active quest");
        assert_eq!(
            unwound.id, manual_id,
            "after reminder quest resolves, Main should unwind to previous valid override"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_order_is_overdue_then_priority_then_oldest() {
        let db_path = temp_db_path("fallback-order-overdue-priority-oldest");
        let mut conn = open_db_at_path(&db_path);

        let overdue_low = insert_active_quest(
            &mut conn,
            "overdue-low",
            1,
            "2026-03-01T08:00:00Z",
            Some("2000-01-01"),
            None,
        );
        let overdue_urgent = insert_active_quest(
            &mut conn,
            "overdue-urgent",
            4,
            "2026-03-02T08:00:00Z",
            Some("2000-01-02"),
            None,
        );
        let future_urgent = insert_active_quest(
            &mut conn,
            "future-urgent",
            4,
            "2026-03-03T08:00:00Z",
            Some("2999-01-01"),
            None,
        );

        let first = resolve_active_quest(&mut conn)
            .expect("resolve with mixed overdue state")
            .expect("must return active quest");
        assert_eq!(
            first.id, overdue_urgent,
            "among overdue quests, higher priority should win"
        );

        diesel::update(quests::table.find(&overdue_urgent))
            .set(quests::status.eq("completed"))
            .execute(&mut conn)
            .expect("complete overdue urgent quest");

        let second = resolve_active_quest(&mut conn)
            .expect("resolve after completing overdue urgent")
            .expect("must return active quest");
        assert_eq!(
            second.id, overdue_low,
            "remaining overdue quest should still beat non-overdue urgent quest"
        );

        diesel::update(quests::table.find(&overdue_low))
            .set(quests::status.eq("completed"))
            .execute(&mut conn)
            .expect("complete overdue low quest");

        let third = resolve_active_quest(&mut conn)
            .expect("resolve after clearing overdue quests")
            .expect("must return active quest");
        assert_eq!(
            third.id, future_urgent,
            "when no overdue quests remain, highest priority fallback should win"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_uses_rank_before_priority() {
        let db_path = temp_db_path("fallback-uses-rank-before-priority");
        let mut conn = open_db_at_path(&db_path);

        let lower_rank_lower_priority = insert_active_quest(
            &mut conn,
            "rank-first-low-priority",
            1,
            "2026-03-01T08:00:00Z",
            None,
            None,
        );
        let higher_rank_higher_priority = insert_active_quest(
            &mut conn,
            "rank-first-high-priority",
            4,
            "2026-03-01T09:00:00Z",
            None,
            None,
        );

        diesel::update(quests::table.find(&lower_rank_lower_priority))
            .set(quests::order_rank.eq(-50.0))
            .execute(&mut conn)
            .expect("set lower rank");
        diesel::update(quests::table.find(&higher_rank_higher_priority))
            .set(quests::order_rank.eq(50.0))
            .execute(&mut conn)
            .expect("set higher rank");

        let winner = resolve_active_quest(&mut conn)
            .expect("resolve active after rank updates")
            .expect("must return active quest");

        assert_eq!(
            winner.id, lower_rank_lower_priority,
            "rank ordering must beat priority when no overdue or focus overrides exist"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_uses_oldest_created_at_on_tie() {
        let db_path = temp_db_path("fallback-uses-oldest-created-at-on-tie");
        let mut conn = open_db_at_path(&db_path);

        let oldest = insert_active_quest(
            &mut conn,
            "tie-oldest",
            3,
            "2026-03-01T08:00:00Z",
            None,
            None,
        );
        let newest = insert_active_quest(
            &mut conn,
            "tie-newest",
            3,
            "2026-03-02T08:00:00Z",
            None,
            None,
        );

        let winner = resolve_active_quest(&mut conn)
            .expect("resolve tie on fallback")
            .expect("must return active quest");

        assert_eq!(
            winner.id, oldest,
            "with equal overdue and priority, oldest created_at should win"
        );
        assert_ne!(winner.id, newest);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn legacy_v2_db_migrates_to_text_ids_without_data_loss() {
        let db_path = temp_db_path("legacy-v2-db-migrates-to-text-ids");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open legacy db path");

        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys on legacy db");

        execute_sql_script(
            &mut conn,
            include_str!("../migrations/00000000000001_init/up.sql"),
        );
        execute_sql_script(
            &mut conn,
            include_str!("../migrations/00000000000002_quest_model_v2/up.sql"),
        );

        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
                version VARCHAR(50) PRIMARY KEY NOT NULL,
                run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut conn)
        .expect("create migrations metadata table");
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES
                ('00000000000001'),
                ('00000000000002')",
        )
        .execute(&mut conn)
        .expect("seed applied legacy migration versions");

        diesel::sql_query(
            "INSERT INTO spaces (id, name, item_order) VALUES (7, 'Legacy Custom', 7)",
        )
        .execute(&mut conn)
        .expect("insert legacy custom space");
        diesel::sql_query(
            "INSERT INTO quests (id, space_id, title, description, status, energy, priority, pinned, created_at, updated_at)
             VALUES (55, 7, 'Legacy Quest', 'before migration', 'active', 'medium', 1, 0, datetime('now'), datetime('now'))",
        )
        .execute(&mut conn)
        .expect("insert legacy quest row");

        conn.run_pending_migrations(MIGRATIONS)
            .expect("run pending migrations from legacy to latest");

        let legacy_space_id: String = spaces::table
            .filter(spaces::name.eq("Legacy Custom"))
            .select(spaces::id)
            .first(&mut conn)
            .expect("load migrated legacy space id");
        assert_ne!(legacy_space_id, "1");
        assert_ne!(legacy_space_id, "2");
        assert_ne!(legacy_space_id, "3");
        assert!(
            is_uuid_like(&legacy_space_id),
            "legacy custom space id must become UUID"
        );

        let migrated_rows: Vec<(String, String, Option<String>)> = quests::table
            .filter(quests::title.eq("Legacy Quest"))
            .select((quests::id, quests::space_id, quests::description))
            .load(&mut conn)
            .expect("load migrated legacy quest");

        assert_eq!(
            migrated_rows.len(),
            1,
            "legacy quest must still exist after migration"
        );
        let (quest_id, quest_space_id, description) = &migrated_rows[0];
        assert!(is_uuid_like(quest_id), "legacy quest id must become UUID");
        assert_eq!(
            quest_space_id, &legacy_space_id,
            "legacy quest must keep space membership through mapping"
        );
        assert_eq!(
            description.as_deref(),
            Some("before migration"),
            "legacy quest payload must be preserved"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn legacy_v2_custom_work_space_migrates_without_duplicate_work() {
        let db_path = temp_db_path("legacy-v2-custom-work-space-migrates-without-duplicate-work");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open legacy db path");

        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys on legacy db");

        execute_sql_script(
            &mut conn,
            include_str!("../migrations/00000000000001_init/up.sql"),
        );
        execute_sql_script(
            &mut conn,
            include_str!("../migrations/00000000000002_quest_model_v2/up.sql"),
        );

        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
                version VARCHAR(50) PRIMARY KEY NOT NULL,
                run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut conn)
        .expect("create migrations metadata table");
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES
                ('00000000000001'),
                ('00000000000002')",
        )
        .execute(&mut conn)
        .expect("seed applied legacy migration versions");

        diesel::sql_query("INSERT INTO spaces (id, name, item_order) VALUES (2, 'Work', 2)")
            .execute(&mut conn)
            .expect("insert legacy custom work space");
        diesel::sql_query(
            "INSERT INTO quests (id, space_id, title, status, energy, priority, pinned, created_at, updated_at)
             VALUES (77, 2, 'Legacy Work Quest', 'active', 'medium', 1, 0, datetime('now'), datetime('now'))",
        )
        .execute(&mut conn)
        .expect("insert quest in legacy custom work space");

        conn.run_pending_migrations(MIGRATIONS)
            .expect("run pending migrations from legacy to latest");

        let work_count = spaces::table
            .filter(spaces::name.eq("Work"))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("count Work spaces");
        let family_count = spaces::table
            .filter(spaces::name.eq("Family"))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("count Family spaces");

        assert_eq!(
            work_count, 1,
            "migration must not create duplicate Work spaces"
        );
        assert_eq!(family_count, 1, "migration must produce one Family space");

        let quest_space_id: String = quests::table
            .filter(quests::title.eq("Legacy Work Quest"))
            .select(quests::space_id)
            .first(&mut conn)
            .expect("load migrated work quest space id");
        assert_eq!(
            quest_space_id, "3",
            "legacy Work-named custom space should map to built-in Work id=3"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn repair_migration_fixes_duplicate_work_from_buggy_v4_state() {
        let db_path = temp_db_path("repair-migration-fixes-duplicate-work-from-buggy-v4-state");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open post-v4 db path");

        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys");

        diesel::sql_query(
            "CREATE TABLE spaces (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                item_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&mut conn)
        .expect("create spaces table for simulated v4 state");

        diesel::sql_query(
            "CREATE TABLE quests (
                id TEXT PRIMARY KEY NOT NULL,
                space_id TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                energy TEXT NOT NULL DEFAULT 'medium',
                priority INTEGER NOT NULL DEFAULT 1,
                pinned BOOLEAN NOT NULL DEFAULT 0,
                due TEXT,
                due_time TEXT,
                repeat_rule TEXT,
                completed_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&mut conn)
        .expect("create quests table for simulated v4 state");

        diesel::sql_query(
            "CREATE TABLE __diesel_schema_migrations (
                version VARCHAR(50) PRIMARY KEY NOT NULL,
                run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut conn)
        .expect("create migrations metadata table");
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES
                ('00000000000001'),
                ('00000000000002'),
                ('00000000000003'),
                ('00000000000004')",
        )
        .execute(&mut conn)
        .expect("seed metadata to simulated v4 state");

        diesel::insert_into(spaces::table)
            .values(&vec![
                (
                    spaces::id.eq("1"),
                    spaces::name.eq("Personal"),
                    spaces::item_order.eq(0_i64),
                ),
                (
                    spaces::id.eq("2"),
                    spaces::name.eq("Work"),
                    spaces::item_order.eq(1_i64),
                ),
                (
                    spaces::id.eq("3"),
                    spaces::name.eq("Work"),
                    spaces::item_order.eq(2_i64),
                ),
            ])
            .execute(&mut conn)
            .expect("insert simulated duplicate-Work spaces");

        conn.run_pending_migrations(MIGRATIONS)
            .expect("apply repair migration");

        let id2_name: String = spaces::table
            .find("2")
            .select(spaces::name)
            .first(&mut conn)
            .expect("load repaired id=2 name");
        let work_count = spaces::table
            .filter(spaces::name.eq("Work"))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("count Work spaces after repair");

        assert_eq!(
            id2_name, "Family",
            "repair migration must convert duplicate Work in id=2 to Family"
        );
        assert_eq!(
            work_count, 1,
            "repair migration must leave a single Work space"
        );

        let _ = std::fs::remove_file(db_path);
    }
}
