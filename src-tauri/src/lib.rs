mod schema;
// mod voice;       // postponed
// mod model_download; // postponed

use schema::{quests, spaces};

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Manager, State};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub struct DbState(pub Mutex<SqliteConnection>);

fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

// ── DB models (Queryable) ─────────────────────────────────────────────────────

#[derive(Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = spaces)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Space {
    pub id: i64,
    pub name: String,
    pub item_order: i64,
    pub created_at: String,
}

#[derive(Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = quests)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Quest {
    pub id: i64,
    pub space_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub energy_required: Option<i64>,
    /// 1 = none, 2 = low, 3 = medium, 4 = urgent
    pub priority: i64,
    pub pinned: bool,
    pub due: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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
    pub space_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub energy_required: Option<i64>,
    #[serde(default = "default_priority")]
    pub priority: i64,
    pub due: Option<String>,
}

fn default_priority() -> i64 { 1 }
fn default_space_id() -> Option<i64> { Some(1) }

#[derive(Deserialize, AsChangeset)]
#[diesel(table_name = quests)]
pub struct UpdateQuestInput {
    pub space_id: Option<i64>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub energy_required: Option<i64>,
    pub priority: Option<i64>,
    pub pinned: Option<bool>,
    pub due: Option<String>,
}

// ── DB setup ──────────────────────────────────────────────────────────────────

fn open_db(app: &tauri::AppHandle) -> SqliteConnection {
    let data_dir = app
        .path()
        .app_data_dir()
        .expect("failed to resolve app data dir");
    std::fs::create_dir_all(&data_dir).expect("failed to create app data dir");
    let db_path = data_dir.join("fini.db");
    let mut conn = SqliteConnection::establish(db_path.to_str().unwrap())
        .expect("failed to open database");
    diesel::sql_query("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .execute(&mut conn)
        .expect("failed to set PRAGMAs");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("failed to run migrations");
    conn
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
fn update_space(state: State<DbState>, id: i64, input: UpdateSpaceInput) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::update(spaces::table.find(id))
        .set(&input)
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    spaces::table
        .find(id)
        .select(Space::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_space(state: State<DbState>, id: i64) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(spaces::table.find(id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Quest commands ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_quests(state: State<DbState>) -> Result<Vec<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    quests::table
        .select(Quest::as_select())
        .order((quests::pinned.desc(), quests::priority.desc(), quests::created_at.desc()))
        .load(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_quest(state: State<DbState>, input: CreateQuestInput) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::insert_into(quests::table)
        .values(&input)
        .returning(Quest::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_quest(state: State<DbState>, id: i64, input: UpdateQuestInput) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let completed_at = input.status.as_deref().map(|s| {
        if s == "completed" { Some(utc_now()) } else { None }
    });
    diesel::update(quests::table.find(id))
        .set((&input, quests::updated_at.eq(utc_now())))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    if let Some(val) = completed_at {
        diesel::update(quests::table.find(id))
            .set(quests::completed_at.eq(val))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    }
    quests::table
        .find(id)
        .select(Quest::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_quest(state: State<DbState>, id: i64) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(quests::table.find(id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_spaces,
            create_space,
            update_space,
            delete_space,
            get_quests,
            create_quest,
            update_quest,
            delete_quest,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
