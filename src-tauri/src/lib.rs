pub mod mcp;
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

#[derive(Queryable, Selectable, Serialize, Deserialize)]
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
    quests::table
        .select(Quest::as_select())
        .order((
            quests::pinned.desc(),
            quests::priority.desc(),
            quests::created_at.desc(),
        ))
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
fn update_quest(
    state: State<DbState>,
    id: String,
    input: UpdateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let completed_at = input.status.as_deref().map(|s| {
        if s == "completed" {
            Some(utc_now())
        } else {
            None
        }
    });
    diesel::update(quests::table.find(&id))
        .set((&input, quests::updated_at.eq(utc_now())))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    if let Some(val) = completed_at {
        diesel::update(quests::table.find(&id))
            .set(quests::completed_at.eq(val))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    }
    quests::table
        .find(&id)
        .select(Quest::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_quest(state: State<DbState>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(quests::table.find(&id))
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
