use diesel::migration::MigrationSource;
use diesel::prelude::*;
use diesel::sqlite::Sqlite;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
#[cfg(any(feature = "ui-plane", test))]
use std::sync::Mutex;

use crate::services::update_recovery::unsupported_schema_guidance;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");
#[cfg(target_os = "android")]
pub const APP_DATA_DIR_NAME: &str = "com.fini.app";
#[cfg(not(target_os = "android"))]
pub const APP_DATA_DIR_NAME: &str = "fini";

#[cfg(any(feature = "ui-plane", test))]
pub struct AppDbConnection(pub Mutex<SqliteConnection>);

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

#[cfg(any(feature = "ui-plane", test))]
pub fn app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("FINI_APP_DATA_DIR") {
        return PathBuf::from(p);
    }

    if std::env::var_os("FLATPAK_ID").is_some() {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join(APP_DATA_DIR_NAME);
        }
    }

    use tauri::Manager;
    app.path()
        .app_data_dir()
        .expect("failed to resolve app data dir")
}

pub fn open_db_at_path(path: &Path) -> SqliteConnection {
    try_open_db_at_path(path).expect("failed to open compatible Fini database")
}

pub fn try_open_db_at_path(path: &Path) -> Result<SqliteConnection, String> {
    let mut conn = SqliteConnection::establish(path.to_str().ok_or("database path is not UTF-8")?)
        .map_err(|err| format!("failed to open database: {err}"))?;
    diesel::sql_query(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;",
    )
    .execute(&mut conn)
    .map_err(|err| format!("failed to set database PRAGMAs: {err}"))?;
    ensure_database_schema_is_supported(&mut conn)?;
    conn.run_pending_migrations(MIGRATIONS)
        .map_err(|err| format!("failed to run database migrations: {err}"))?;
    Ok(conn)
}

fn ensure_database_schema_is_supported(conn: &mut SqliteConnection) -> Result<(), String> {
    let known_versions = embedded_migration_versions()?;
    let applied_versions = conn
        .applied_migrations()
        .map_err(|err| format!("failed to read database migration state: {err}"))?;

    if let Some(version) = applied_versions
        .iter()
        .map(ToString::to_string)
        .find(|version| !known_versions.contains(version))
    {
        return Err(format!(
            "database schema is not supported by this Fini binary. Fini version: {}; unknown database migration: {version}. {}",
            env!("CARGO_PKG_VERSION"),
            unsupported_schema_guidance()
        ));
    }

    Ok(())
}

fn embedded_migration_versions() -> Result<BTreeSet<String>, String> {
    <EmbeddedMigrations as MigrationSource<Sqlite>>::migrations(&MIGRATIONS)
        .map_err(|err| format!("failed to read embedded database migrations: {err}"))
        .map(|migrations| {
            migrations
                .into_iter()
                .map(|migration| migration.name().version().to_string())
                .collect()
        })
}

#[cfg(any(feature = "ui-plane", test))]
pub fn try_open_db(app: &tauri::AppHandle) -> Result<SqliteConnection, String> {
    if std::env::var_os("FINI_DB_PATH").is_some() {
        let db_path = db_default_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create app data dir: {err}"))?;
        }
        return try_open_db_at_path(&db_path);
    }

    let data_dir = app_data_dir(app);
    std::fs::create_dir_all(&data_dir)
        .map_err(|err| format!("failed to create app data dir: {err}"))?;
    let db_path = data_dir.join("fini.db");
    try_open_db_at_path(&db_path)
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
    use crate::schema::{quest_series, quests, spaces};
    use diesel::connection::SimpleConnection;

    fn execute_sql_script(conn: &mut SqliteConnection, script: &str) {
        conn.batch_execute(script)
            .expect("execute SQL migration script");
    }

    #[test]
    fn v21_down_migration_restores_legacy_energy_and_priority_schema() {
        let db_path = temp_db_path("v21-energy-priority-down-contract");
        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open v21 db path");
        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys");

        for script in [
            include_str!("../../migrations/00000000000001_init/up.sql"),
            include_str!("../../migrations/00000000000002_quest_model_v2/up.sql"),
            include_str!("../../migrations/00000000000003_quest_space_not_null/up.sql"),
            include_str!("../../migrations/00000000000004_identity_text_ids/up.sql"),
            include_str!("../../migrations/00000000000005_repair_builtin_spaces/up.sql"),
            include_str!("../../migrations/00000000000006_main_focus_events/up.sql"),
            include_str!("../../migrations/00000000000007_quest_order_rank/up.sql"),
            include_str!("../../migrations/00000000000008_quest_series/up.sql"),
            include_str!("../../migrations/00000000000009_reminders/up.sql"),
            include_str!("../../migrations/00000000000010_sync_and_focus/up.sql"),
            include_str!("../../migrations/00000000000011_pair_mapping_last_synced/up.sql"),
            include_str!("../../migrations/00000000000012_focus_history_as_source_of_truth/up.sql"),
            include_str!("../../migrations/00000000000013_reminder_scheduling/up.sql"),
            include_str!("../../migrations/00000000000014_settings/up.sql"),
            include_str!("../../migrations/00000000000015_pair_mapping_end_of_sync/up.sql"),
            include_str!("../../migrations/00000000000016_notification_snoozes/up.sql"),
            include_str!("../../migrations/00000000000017_focus_enter_count/up.sql"),
            include_str!("../../migrations/00000000000018_quest_checklist_md/up.sql"),
            include_str!(
                "../../migrations/00000000000019_paired_device_bluetooth_transport/up.sql"
            ),
            include_str!(
                "../../migrations/00000000000020_paired_device_bluetooth_disabled_by_user/up.sql"
            ),
            include_str!("../../migrations/00000000000021_energy_priority_contract/up.sql"),
        ] {
            execute_sql_script(&mut conn, script);
        }
        diesel::sql_query(
            "INSERT INTO quest_series (id, space_id, title, repeat_rule, energy, priority) VALUES
                ('down-series-small-low', '1', 'small-low', '{\"preset\":\"daily\"}', 1, 1),
                ('down-series-large-high', '1', 'large-high', '{\"preset\":\"daily\"}', 3, 3)",
        )
        .execute(&mut conn)
        .expect("seed canonical series");
        diesel::sql_query(
            "INSERT INTO quests (id, space_id, title, status, energy, priority) VALUES
                ('down-quest-small-low', '1', 'small-low', 'active', 1, 1),
                ('down-quest-large-high', '1', 'large-high', 'active', 3, 3)",
        )
        .execute(&mut conn)
        .expect("seed canonical quests");

        execute_sql_script(
            &mut conn,
            include_str!("../../migrations/00000000000021_energy_priority_contract/down.sql"),
        );

        #[derive(diesel::QueryableByName)]
        struct TableColumn {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            column_type: String,
        }
        #[derive(diesel::QueryableByName)]
        struct LegacyQuestMetadata {
            #[diesel(sql_type = diesel::sql_types::Text)]
            id: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            energy: String,
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            priority: i64,
        }

        let quest_types: Vec<(String, String)> =
            diesel::sql_query("SELECT name, type AS column_type FROM pragma_table_info('quests')")
                .load::<TableColumn>(&mut conn)
                .expect("inspect legacy quest columns")
                .into_iter()
                .filter(|column| column.name == "energy" || column.name == "priority")
                .map(|column| (column.name, column.column_type))
                .collect();
        assert_eq!(
            quest_types,
            vec![
                ("energy".to_string(), "TEXT".to_string()),
                ("priority".to_string(), "INTEGER".to_string())
            ]
        );
        #[derive(diesel::QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        let transient_tables: CountRow = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('quest_series_replacement', 'quests_replacement')",
        )
        .get_result(&mut conn)
        .expect("inspect transient migration tables");
        assert_eq!(
            transient_tables.count, 0,
            "migration must not leave replacement tables"
        );

        let rows: Vec<LegacyQuestMetadata> =
            diesel::sql_query("SELECT id, energy, priority FROM quests ORDER BY id")
                .load(&mut conn)
                .expect("load legacy-shaped quest values");
        assert_eq!(
            rows.into_iter()
                .map(|row| (row.id, row.energy, row.priority))
                .collect::<Vec<_>>(),
            vec![
                ("down-quest-large-high".into(), "high".into(), 4),
                ("down-quest-small-low".into(), "low".into(), 2),
            ]
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn v20_energy_priority_rows_migrate_to_constrained_integer_contract() {
        let db_path = temp_db_path("v20-energy-priority-contract");
        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open v20 db path");
        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys");

        // Apply the real historical DDL, then mark it applied so v21 is the only pending
        // migration. This protects the real v20 upgrade path, not just fresh databases.
        for script in [
            include_str!("../../migrations/00000000000001_init/up.sql"),
            include_str!("../../migrations/00000000000002_quest_model_v2/up.sql"),
            include_str!("../../migrations/00000000000003_quest_space_not_null/up.sql"),
            include_str!("../../migrations/00000000000004_identity_text_ids/up.sql"),
            include_str!("../../migrations/00000000000005_repair_builtin_spaces/up.sql"),
            include_str!("../../migrations/00000000000006_main_focus_events/up.sql"),
            include_str!("../../migrations/00000000000007_quest_order_rank/up.sql"),
            include_str!("../../migrations/00000000000008_quest_series/up.sql"),
            include_str!("../../migrations/00000000000009_reminders/up.sql"),
            include_str!("../../migrations/00000000000010_sync_and_focus/up.sql"),
            include_str!("../../migrations/00000000000011_pair_mapping_last_synced/up.sql"),
            include_str!("../../migrations/00000000000012_focus_history_as_source_of_truth/up.sql"),
            include_str!("../../migrations/00000000000013_reminder_scheduling/up.sql"),
            include_str!("../../migrations/00000000000014_settings/up.sql"),
            include_str!("../../migrations/00000000000015_pair_mapping_end_of_sync/up.sql"),
            include_str!("../../migrations/00000000000016_notification_snoozes/up.sql"),
            include_str!("../../migrations/00000000000017_focus_enter_count/up.sql"),
            include_str!("../../migrations/00000000000018_quest_checklist_md/up.sql"),
            include_str!(
                "../../migrations/00000000000019_paired_device_bluetooth_transport/up.sql"
            ),
            include_str!(
                "../../migrations/00000000000020_paired_device_bluetooth_disabled_by_user/up.sql"
            ),
        ] {
            execute_sql_script(&mut conn, script);
        }
        diesel::sql_query(
            "CREATE TABLE __diesel_schema_migrations (
                version VARCHAR(50) PRIMARY KEY NOT NULL,
                run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut conn)
        .expect("create migration metadata");
        for version in 1..=20 {
            diesel::sql_query(format!(
                "INSERT INTO __diesel_schema_migrations (version) VALUES ('000000000000{version:02}')"
            ))
            .execute(&mut conn)
            .expect("mark historical migration applied");
        }
        diesel::sql_query(
            "INSERT INTO quest_series (id, space_id, title, repeat_rule, priority, energy) VALUES
                ('series-low', '1', 'low', '{\"preset\":\"daily\"}', 'low', 'low'),
                ('series-mid', '1', 'mid', '{\"preset\":\"daily\"}', 'medium', 'medium'),
                ('series-high', '1', 'high', '{\"preset\":\"daily\"}', 'urgent', 'high'),
                ('series-fallback', '1', 'fallback', '{\"preset\":\"daily\"}', 'none', 'unknown')",
        )
        .execute(&mut conn)
        .expect("seed v20 series metadata");
        diesel::sql_query(
            "INSERT INTO quests (id, space_id, title, status, priority, energy) VALUES
                ('quest-low', '1', 'low', 'active', 'low', 'low'),
                ('quest-mid', '1', 'mid', 'active', 'medium', 'medium'),
                ('quest-high', '1', 'high', 'active', 'urgent', 'high'),
                ('quest-fallback', '1', 'fallback', 'active', 'none', 'unknown')",
        )
        .execute(&mut conn)
        .expect("seed v20 quest metadata");
        conn.batch_execute(
            "UPDATE quests SET series_id = 'series-low' WHERE id = 'quest-low';
             INSERT INTO reminders (id, quest_id, type) VALUES ('migration-reminder', 'quest-low', 'relative');
             INSERT INTO focus_history (id, quest_id, space_id, trigger) VALUES
                ('migration-focus-history', 'quest-low', '1', 'manual');
             INSERT INTO series_reminder_templates (id, series_id, kind) VALUES
                ('migration-series-reminder', 'series-low', 'relative');",
        )
        .expect("seed v20 dependent rows");

        conn.run_pending_migrations(MIGRATIONS)
            .expect("migrate v20 database through v21");
        let quest_values: Vec<(String, i64, i64)> = quests::table
            .select((quests::id, quests::energy, quests::priority))
            .order(quests::id.asc())
            .load(&mut conn)
            .expect("load migrated quest metadata");
        let series_values: Vec<(String, i64, i64)> = quest_series::table
            .select((
                quest_series::id,
                quest_series::energy,
                quest_series::priority,
            ))
            .order(quest_series::id.asc())
            .load(&mut conn)
            .expect("load migrated series metadata");
        assert_eq!(
            quest_values,
            vec![
                ("quest-fallback".into(), 2, 2),
                ("quest-high".into(), 3, 3),
                ("quest-low".into(), 1, 1),
                ("quest-mid".into(), 2, 2),
            ]
        );
        assert_eq!(
            series_values,
            vec![
                ("series-fallback".into(), 2, 2),
                ("series-high".into(), 3, 3),
                ("series-low".into(), 1, 1),
                ("series-mid".into(), 2, 2),
            ]
        );
        #[derive(diesel::QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }
        for (table, id) in [
            ("reminders", "migration-reminder"),
            ("focus_history", "migration-focus-history"),
            ("series_reminder_templates", "migration-series-reminder"),
        ] {
            let count: i64 = diesel::sql_query(format!(
                "SELECT COUNT(*) AS count FROM {table} WHERE id = '{id}'"
            ))
            .get_result::<CountRow>(&mut conn)
            .expect("load dependent row count")
            .count;
            assert_eq!(count, 1, "v21 migration must preserve {table} rows");
        }
        let series_id: Option<String> = quests::table
            .find("quest-low")
            .select(quests::series_id)
            .first(&mut conn)
            .expect("load migrated recurring quest series id");
        assert_eq!(series_id.as_deref(), Some("series-low"));
        let violations: CountRow =
            diesel::sql_query("SELECT COUNT(*) AS count FROM pragma_foreign_key_check")
                .get_result(&mut conn)
                .expect("check foreign keys");
        assert_eq!(violations.count, 0, "migration must preserve foreign keys");
        let _ = std::fs::remove_file(db_path);
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
    fn database_with_unknown_migration_is_rejected() {
        let db_path = temp_db_path("database-with-unknown-migration-is-rejected");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open db path");

        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
                version VARCHAR(50) PRIMARY KEY NOT NULL,
                run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut conn)
        .expect("create migrations metadata table");
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('99999999999999')",
        )
        .execute(&mut conn)
        .expect("seed unknown future migration version");
        drop(conn);

        let err = match try_open_db_at_path(&db_path) {
            Ok(_) => panic!("database with unknown migration must be rejected"),
            Err(err) => err,
        };

        assert!(
            err.contains("database schema is not supported by this Fini binary"),
            "error should explain unsupported schema, got: {err}"
        );
        assert!(
            err.contains("Fini version:"),
            "error should include Fini version, got: {err}"
        );
        assert!(
            err.contains("99999999999999"),
            "error should include unknown migration version, got: {err}"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn v2_numeric_id_db_migrates_to_text_ids_without_data_loss() {
        let db_path = temp_db_path("v2-numeric-id-db-migrates-to-text-ids");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open v2 db path");

        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys on v2 db");

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
        .expect("seed applied v2 migration versions");

        diesel::sql_query("INSERT INTO spaces (id, name, item_order) VALUES (7, 'V2 Custom', 7)")
            .execute(&mut conn)
            .expect("insert v2 custom space");
        diesel::sql_query(
            "INSERT INTO quests (id, space_id, title, description, status, energy, priority, pinned, created_at, updated_at)
             VALUES (55, 7, 'V2 Quest', 'before migration', 'active', 'medium', 1, 0, datetime('now'), datetime('now'))",
        )
        .execute(&mut conn)
        .expect("insert v2 quest row");
        drop(conn);

        let mut conn = open_db_at_path(&db_path);

        let migrated_space_id: String = spaces::table
            .filter(spaces::name.eq("V2 Custom"))
            .select(spaces::id)
            .first(&mut conn)
            .expect("load migrated v2 space id");
        assert_ne!(migrated_space_id, "1");
        assert_ne!(migrated_space_id, "2");
        assert_ne!(migrated_space_id, "3");
        assert!(
            is_uuid_like(&migrated_space_id),
            "v2 custom space id must become UUID"
        );

        let migrated_rows: Vec<(String, String, Option<String>)> = quests::table
            .filter(quests::title.eq("V2 Quest"))
            .select((quests::id, quests::space_id, quests::description))
            .load(&mut conn)
            .expect("load migrated v2 quest");

        assert_eq!(
            migrated_rows.len(),
            1,
            "v2 quest must still exist after migration"
        );
        let (quest_id, quest_space_id, description) = &migrated_rows[0];
        assert!(is_uuid_like(quest_id), "v2 quest id must become UUID");
        assert_eq!(
            quest_space_id, &migrated_space_id,
            "v2 quest must keep space membership"
        );
        assert_eq!(
            description.as_deref(),
            Some("before migration"),
            "v2 quest payload must be preserved"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn v2_custom_work_space_migrates_without_duplicate_work() {
        let db_path = temp_db_path("v2-custom-work-space-migrates-without-duplicate-work");

        let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
            .expect("open v2 db path");

        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut conn)
            .expect("enable foreign keys on v2 db");

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
        .expect("seed applied v2 migration versions");

        diesel::sql_query("INSERT INTO spaces (id, name, item_order) VALUES (2, 'Work', 2)")
            .execute(&mut conn)
            .expect("insert v2 custom work space");
        diesel::sql_query(
            "INSERT INTO quests (id, space_id, title, status, energy, priority, pinned, created_at, updated_at)
             VALUES (77, 2, 'V2 Work Quest', 'active', 'medium', 1, 0, datetime('now'), datetime('now'))",
        )
        .execute(&mut conn)
        .expect("insert quest in v2 custom work space");

        conn.run_pending_migrations(MIGRATIONS)
            .expect("run pending migrations from v2 state");

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
            .filter(quests::title.eq("V2 Work Quest"))
            .select(quests::space_id)
            .first(&mut conn)
            .expect("load migrated work quest space id");
        assert_eq!(
            quest_space_id, "3",
            "v2 Work-named custom space should map to built-in Work id=3"
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
