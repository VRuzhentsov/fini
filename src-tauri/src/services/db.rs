use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");
#[cfg(target_os = "android")]
pub const APP_DATA_DIR_NAME: &str = "com.fini.app";
#[cfg(not(target_os = "android"))]
pub const APP_DATA_DIR_NAME: &str = "fini";

pub struct DbState(pub Mutex<SqliteConnection>);

pub fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn db_default_path() -> PathBuf {
    if let Ok(p) = std::env::var("FINI_DB_PATH") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .expect("failed to get data dir")
        .join(APP_DATA_DIR_NAME)
        .join("fini.db")
}

pub fn app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("FINI_APP_DATA_DIR") {
        return PathBuf::from(p);
    }
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .expect("failed to resolve app data dir")
}

pub fn open_db_at_path(path: &Path) -> SqliteConnection {
    let mut conn =
        SqliteConnection::establish(path.to_str().unwrap()).expect("failed to open database");
    diesel::sql_query("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .execute(&mut conn)
        .expect("failed to set PRAGMAs");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("failed to run migrations");
    conn
}

pub fn open_db(app: &tauri::AppHandle) -> SqliteConnection {
    let data_dir = app_data_dir(app);
    std::fs::create_dir_all(&data_dir).expect("failed to create app data dir");
    let db_path = data_dir.join("fini.db");
    open_db_at_path(&db_path)
}

#[cfg(test)]
pub fn temp_db_path(label: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    std::env::temp_dir().join(format!("fini-{label}-{unique}.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{quests, spaces};

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

    fn is_uuid_like(value: &str) -> bool {
        value.len() == 36
            && value.as_bytes()[8] == b'-'
            && value.as_bytes()[13] == b'-'
            && value.as_bytes()[18] == b'-'
            && value.as_bytes()[23] == b'-'
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
    fn legacy_v2_db_migrates_to_text_ids_without_data_loss() {
        let db_path = temp_db_path("legacy-v2-db-migrates-to-text-ids");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open legacy db path");

        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys on legacy db");

        execute_sql_script(
            &mut conn,
            include_str!("../../migrations/00000000000001_init/up.sql"),
        );
        execute_sql_script(
            &mut conn,
            include_str!("../../migrations/00000000000002_quest_model_v2/up.sql"),
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
            "legacy quest must keep space membership"
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
            include_str!("../../migrations/00000000000001_init/up.sql"),
        );
        execute_sql_script(
            &mut conn,
            include_str!("../../migrations/00000000000002_quest_model_v2/up.sql"),
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
