use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::Text;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "ui-plane", test))]
use tauri::{AppHandle, State};
#[cfg(any(feature = "ui-plane", test))]
use tauri_plugin_fs::{FilePath, FsExt};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::models::{ChecklistActivity, Quest, QuestSeries, Space};
use crate::schema::{checklist_activity, quest_series, quests, spaces};
use crate::services::db::utc_now;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;

const MANIFEST_NAME: &str = "manifest.json";
const BACKUP_DB_NAME: &str = "fini-backup.sqlite";
const BACKUP_FORMAT: &str = "fini-backup";
const BACKUP_VERSION: u32 = 2;
const BUILTIN_SPACE_IDS: [&str; 3] = ["1", "2", "3"];

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupManifest {
    pub format: String,
    pub version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub domains: Vec<String>,
    pub spaces: Vec<ManifestSpace>,
    pub counts: ManifestCounts,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ManifestSpace {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ManifestCounts {
    pub spaces: usize,
    pub quest_series: usize,
    pub quests: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupExportResult {
    pub path: String,
    pub manifest: BackupManifest,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupArchiveInspection {
    pub valid: bool,
    pub manifest: BackupManifest,
    pub contents: BackupArchiveContents,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupArchiveContents {
    pub spaces: Vec<ManifestSpace>,
    pub counts: ManifestCounts,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupImportPreflight {
    pub manifest: BackupManifest,
    pub required_space_mappings: Vec<BackupSpaceMappingRequest>,
    pub conflicts: Vec<BackupConflict>,
}

/// A read-only archive recovery plan. This describes whether a later import could proceed;
/// it never applies an archive or starts recovery.
#[derive(Debug, Serialize)]
pub struct BackupImportDryRunPlan {
    pub dry_run: bool,
    pub ready_to_apply: bool,
    pub no_apply_or_recovery_action_occurred: bool,
    pub manifest: BackupManifest,
    pub required_space_mappings: Vec<BackupSpaceMappingRequest>,
    pub conflicts: Vec<BackupConflict>,
}

impl BackupImportDryRunPlan {
    pub fn from_preflight(preflight: BackupImportPreflight) -> Self {
        let ready_to_apply =
            preflight.required_space_mappings.is_empty() && preflight.conflicts.is_empty();
        Self {
            dry_run: true,
            ready_to_apply,
            no_apply_or_recovery_action_occurred: true,
            manifest: preflight.manifest,
            required_space_mappings: preflight.required_space_mappings,
            conflicts: preflight.conflicts,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupSpaceMappingRequest {
    pub backup_space_id: String,
    pub backup_space_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupConflict {
    pub entity_type: String,
    pub id: String,
    pub title: String,
    pub local_summary: String,
    pub backup_summary: String,
    pub local: serde_json::Value,
    pub backup: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BackupSpaceMappingInput {
    pub backup_space_id: String,
    pub mode: String,
    pub local_space_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BackupConflictResolutionInput {
    pub entity_type: String,
    pub id: String,
    pub resolution: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupImportResult {
    pub imported: bool,
    pub spaces: usize,
    pub quest_series: usize,
    pub quests: usize,
}

#[derive(QueryableByName)]
struct TableNameRow {
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
struct ColumnNameRow {
    #[diesel(sql_type = Text)]
    name: String,
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn backup_export(
    app: AppHandle,
    state: State<AppDbConnection>,
    path: String,
    space_ids: Vec<String>,
) -> Result<BackupExportResult, String> {
    let file_path: FilePath = path.parse().unwrap();
    let mut conn = state.inner().0.lock().unwrap();
    match file_path {
        FilePath::Path(p) => export_backup(&mut conn, &p, &space_ids),
        FilePath::Url(url) => {
            let temp = temp_zip_path("export")?;
            let result = export_backup(&mut conn, &temp, &space_ids)?;
            let mut opts = tauri_plugin_fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            let mut dst = app
                .fs()
                .open(FilePath::Url(url.clone()), opts)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut File::open(&temp).map_err(|e| e.to_string())?, &mut dst)
                .map_err(|e| e.to_string())?;
            fs::remove_file(&temp).ok();
            Ok(BackupExportResult {
                path: url.to_string(),
                manifest: result.manifest,
            })
        }
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn backup_preflight_import(
    app: AppHandle,
    state: State<AppDbConnection>,
    path: String,
    mappings: Vec<BackupSpaceMappingInput>,
) -> Result<BackupImportPreflight, String> {
    let file_path: FilePath = path.parse().unwrap();
    let mut conn = state.inner().0.lock().unwrap();
    match file_path {
        FilePath::Path(p) => preflight_import(&mut conn, &p, &mappings),
        FilePath::Url(url) => {
            let temp = copy_content_uri_to_temp(&app, FilePath::Url(url))?;
            let result = preflight_import(&mut conn, &temp, &mappings);
            fs::remove_file(&temp).ok();
            result
        }
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn backup_apply_import(
    app: AppHandle,
    state: State<AppDbConnection>,
    path: String,
    mappings: Vec<BackupSpaceMappingInput>,
    resolutions: Vec<BackupConflictResolutionInput>,
) -> Result<BackupImportResult, String> {
    let file_path: FilePath = path.parse().unwrap();
    let mut conn = state.inner().0.lock().unwrap();
    match file_path {
        FilePath::Path(p) => apply_import(&mut conn, &p, &mappings, &resolutions),
        FilePath::Url(url) => {
            let temp = copy_content_uri_to_temp(&app, FilePath::Url(url))?;
            let result = apply_import(&mut conn, &temp, &mappings, &resolutions);
            fs::remove_file(&temp).ok();
            result
        }
    }
}

#[cfg(any(feature = "ui-plane", test))]
fn copy_content_uri_to_temp(app: &AppHandle, file_path: FilePath) -> Result<PathBuf, String> {
    let mut opts = tauri_plugin_fs::OpenOptions::new();
    opts.read(true);
    let mut src = app.fs().open(file_path, opts).map_err(|e| e.to_string())?;
    let temp_path = std::env::temp_dir().join(format!("fini-backup-import-{}.zip", Uuid::new_v4()));
    let mut dest = File::create(&temp_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut src, &mut dest).map_err(|e| e.to_string())?;
    Ok(temp_path)
}

fn temp_zip_path(label: &str) -> Result<PathBuf, String> {
    Ok(std::env::temp_dir().join(format!("fini-backup-{label}-{}.zip", Uuid::new_v4())))
}

pub fn export_backup(
    conn: &mut SqliteConnection,
    output_path: &Path,
    space_ids: &[String],
) -> Result<BackupExportResult, String> {
    if space_ids.is_empty() {
        return Err("select at least one space to export".to_string());
    }

    let selected: HashSet<&str> = space_ids.iter().map(String::as_str).collect();
    let selected_spaces = spaces::table
        .filter(spaces::id.eq_any(space_ids))
        .select(Space::as_select())
        .order(spaces::item_order.asc())
        .load::<Space>(conn)
        .map_err(|e| e.to_string())?;

    if selected_spaces.len() != selected.len() {
        return Err("one or more selected spaces were not found".to_string());
    }

    let selected_series = quest_series::table
        .filter(quest_series::space_id.eq_any(space_ids))
        .select(QuestSeries::as_select())
        .load::<QuestSeries>(conn)
        .map_err(|e| e.to_string())?;
    let selected_quests = quests::table
        .filter(quests::space_id.eq_any(space_ids))
        .select(Quest::as_select())
        .load::<Quest>(conn)
        .map_err(|e| e.to_string())?;
    let selected_quest_ids = selected_quests
        .iter()
        .map(|quest| quest.id.clone())
        .collect::<Vec<_>>();
    let selected_checklist_activity = checklist_activity::table
        .filter(checklist_activity::quest_id.eq_any(&selected_quest_ids))
        .select(ChecklistActivity::as_select())
        .load::<ChecklistActivity>(conn)
        .map_err(|e| e.to_string())?;

    let manifest = BackupManifest {
        format: BACKUP_FORMAT.to_string(),
        version: BACKUP_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: utc_now(),
        domains: vec![
            "spaces".to_string(),
            "quest_series".to_string(),
            "quests".to_string(),
        ],
        spaces: selected_spaces
            .iter()
            .map(|space| ManifestSpace {
                id: space.id.clone(),
                name: space.name.clone(),
            })
            .collect(),
        counts: ManifestCounts {
            spaces: selected_spaces.len(),
            quest_series: selected_series.len(),
            quests: selected_quests.len(),
        },
    };

    let temp_dir = create_temp_dir("export")?;
    let backup_db_path = temp_dir.join(BACKUP_DB_NAME);
    let mut backup_conn = SqliteConnection::establish(path_str(&backup_db_path)?)
        .map_err(|e| format!("failed to create backup database: {e}"))?;
    create_backup_schema(&mut backup_conn)?;

    for space in &selected_spaces {
        insert_space(&mut backup_conn, space).map_err(|e| e.to_string())?;
    }
    for series in &selected_series {
        insert_series(&mut backup_conn, series).map_err(|e| e.to_string())?;
    }
    for quest in &selected_quests {
        insert_quest(&mut backup_conn, quest).map_err(|e| e.to_string())?;
    }
    for activity in &selected_checklist_activity {
        insert_checklist_activity(&mut backup_conn, activity).map_err(|e| e.to_string())?;
    }

    write_zip(output_path, &manifest, &backup_db_path)?;
    let _ = fs::remove_dir_all(&temp_dir);

    Ok(BackupExportResult {
        path: output_path.to_string_lossy().to_string(),
        manifest,
    })
}

pub fn preflight_import(
    conn: &mut SqliteConnection,
    path: &Path,
    mappings: &[BackupSpaceMappingInput],
) -> Result<BackupImportPreflight, String> {
    with_extracted_backup(path, |extracted| {
        let mut backup_conn = SqliteConnection::establish(path_str(&extracted.db_path)?)
            .map_err(|e| format!("failed to open backup database: {e}"))?;
        validate_backup_schema(&mut backup_conn)?;
        migrate_backup_schema(&mut backup_conn)?;
        validate_manifest(&extracted.manifest)?;

        let backup_spaces = load_backup_spaces(&mut backup_conn)?;
        let space_map = resolve_space_map(conn, &backup_spaces, mappings)?;
        let required_space_mappings = required_space_mappings(conn, &backup_spaces, mappings)?;
        let conflicts = collect_conflicts(conn, &mut backup_conn, &space_map)?;

        Ok(BackupImportPreflight {
            manifest: extracted.manifest.clone(),
            required_space_mappings,
            conflicts,
        })
    })
}

/// Read and validate a backup archive without accessing a local Fini database.
pub fn inspect_backup(path: &Path) -> Result<BackupArchiveInspection, String> {
    let extracted = extract_backup(path)?;
    let result = (|| {
        let mut backup_conn = SqliteConnection::establish(path_str(&extracted.db_path)?)
            .map_err(|e| format!("failed to open backup database: {e}"))?;
        validate_backup_schema(&mut backup_conn)?;
        migrate_backup_schema(&mut backup_conn)?;
        validate_manifest(&extracted.manifest)?;

        let spaces = load_backup_spaces(&mut backup_conn)?;
        let quest_series = load_backup_series(&mut backup_conn)?;
        let quests = load_backup_quests(&mut backup_conn)?;
        Ok(BackupArchiveInspection {
            valid: true,
            manifest: extracted.manifest,
            contents: BackupArchiveContents {
                spaces: spaces
                    .iter()
                    .map(|space| ManifestSpace {
                        id: space.id.clone(),
                        name: space.name.clone(),
                    })
                    .collect(),
                counts: ManifestCounts {
                    spaces: spaces.len(),
                    quest_series: quest_series.len(),
                    quests: quests.len(),
                },
            },
        })
    })();
    let _ = fs::remove_dir_all(&extracted.temp_dir);
    result
}

pub fn apply_import(
    conn: &mut SqliteConnection,
    path: &Path,
    mappings: &[BackupSpaceMappingInput],
    resolutions: &[BackupConflictResolutionInput],
) -> Result<BackupImportResult, String> {
    let preflight = preflight_import(conn, path, mappings)?;
    if !preflight.required_space_mappings.is_empty() {
        return Err("resolve all backup space mappings before import".to_string());
    }

    let resolution_map: HashMap<(String, String), String> = resolutions
        .iter()
        .map(|resolution| {
            (
                (resolution.entity_type.clone(), resolution.id.clone()),
                resolution.resolution.clone(),
            )
        })
        .collect();
    for conflict in &preflight.conflicts {
        let key = (conflict.entity_type.clone(), conflict.id.clone());
        if !matches!(
            resolution_map.get(&key).map(String::as_str),
            Some("local" | "backup")
        ) {
            return Err("resolve every backup conflict before import".to_string());
        }
    }

    with_extracted_backup(path, |extracted| {
        let mut backup_conn = SqliteConnection::establish(path_str(&extracted.db_path)?)
            .map_err(|e| format!("failed to open backup database: {e}"))?;
        validate_backup_schema(&mut backup_conn)?;
        migrate_backup_schema(&mut backup_conn)?;
        validate_manifest(&extracted.manifest)?;

        let backup_spaces = load_backup_spaces(&mut backup_conn)?;
        let backup_series = load_backup_series(&mut backup_conn)?;
        let backup_quests = load_backup_quests(&mut backup_conn)?;
        let backup_checklist_activity = load_backup_checklist_activity(&mut backup_conn)?;
        let space_map = resolve_space_map(conn, &backup_spaces, mappings)?;
        let backup_space_by_id: HashMap<&str, &Space> = backup_spaces
            .iter()
            .map(|space| (space.id.as_str(), space))
            .collect();
        let quest_import_action_by_id = backup_quests
            .iter()
            .map(|quest| {
                let mapped = mapped_quest(quest, &space_map);
                let action = conflict_action(&resolution_map, "quest", &mapped.id);
                (mapped.id, action)
            })
            .collect::<HashMap<_, _>>();

        conn.transaction::<_, diesel::result::Error, _>(|tx| {
            for (backup_space_id, local_space_id) in &space_map {
                if backup_space_id != local_space_id {
                    continue;
                }
                let exists = local_space_exists(tx, local_space_id)?;
                if !exists {
                    if let Some(space) = backup_space_by_id.get(backup_space_id.as_str()) {
                        insert_space(tx, space)?;
                    }
                }
            }

            for series in &backup_series {
                let mapped = mapped_series(series, &space_map);
                let action = conflict_action(&resolution_map, "quest_series", &mapped.id);
                upsert_series_for_import(tx, &mapped, action)?;
            }

            for quest in &backup_quests {
                let mapped = mapped_quest(quest, &space_map);
                let action = conflict_action(&resolution_map, "quest", &mapped.id);
                upsert_quest_for_import(tx, &mapped, action)?;
            }

            for quest in &backup_quests {
                let mapped = mapped_quest(quest, &space_map);
                if matches!(
                    quest_import_action_by_id.get(&mapped.id),
                    Some(ConflictAction::KeepLocal)
                ) {
                    continue;
                }
                delete_checklist_activity_for_quest(tx, &mapped.id)?;
            }

            for activity in &backup_checklist_activity {
                if matches!(
                    quest_import_action_by_id.get(&activity.quest_id),
                    Some(ConflictAction::KeepLocal)
                ) {
                    continue;
                }
                insert_checklist_activity_for_import(tx, activity)?;
            }

            Ok(())
        })
        .map_err(|e| e.to_string())
    })?;
    Ok(BackupImportResult {
        imported: true,
        spaces: preflight.manifest.counts.spaces,
        quest_series: preflight.manifest.counts.quest_series,
        quests: preflight.manifest.counts.quests,
    })
}

#[cfg(feature = "cli-plane")]
pub fn import_cli(
    conn: &mut SqliteConnection,
    path: &Path,
    force: bool,
) -> Result<BackupImportResult, String> {
    let preflight = preflight_import(conn, path, &[])?;
    let mappings: Vec<BackupSpaceMappingInput> = preflight
        .required_space_mappings
        .iter()
        .map(|mapping| BackupSpaceMappingInput {
            backup_space_id: mapping.backup_space_id.clone(),
            mode: "create_new".to_string(),
            local_space_id: None,
        })
        .collect();
    let preflight = preflight_import(conn, path, &mappings)?;
    if !force && !preflight.conflicts.is_empty() {
        return Err(format!(
            "backup import has {} conflict(s); rerun with --force to use backup versions",
            preflight.conflicts.len()
        ));
    }
    let resolutions = preflight
        .conflicts
        .iter()
        .map(|conflict| BackupConflictResolutionInput {
            entity_type: conflict.entity_type.clone(),
            id: conflict.id.clone(),
            resolution: "backup".to_string(),
        })
        .collect::<Vec<_>>();
    apply_import(conn, path, &mappings, &resolutions)
}

struct ExtractedBackup {
    temp_dir: PathBuf,
    db_path: PathBuf,
    manifest: BackupManifest,
}

fn with_extracted_backup<T>(
    path: &Path,
    operation: impl FnOnce(&ExtractedBackup) -> Result<T, String>,
) -> Result<T, String> {
    let extracted = extract_backup(path)?;
    let result = operation(&extracted);
    let _ = fs::remove_dir_all(&extracted.temp_dir);
    result
}

fn create_backup_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    conn.batch_execute(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE spaces (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            item_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        CREATE TABLE quest_series (
            id TEXT PRIMARY KEY NOT NULL,
            space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE SET DEFAULT,
            title TEXT NOT NULL,
            description TEXT,
            repeat_rule TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 1,
            energy TEXT NOT NULL DEFAULT 'medium',
            active BOOLEAN NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            is_checklist BOOLEAN NOT NULL DEFAULT 0
        );
        CREATE TABLE quests (
            id TEXT PRIMARY KEY NOT NULL,
            space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE SET DEFAULT,
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
            order_rank REAL NOT NULL DEFAULT 0,
            focus_enter_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            series_id TEXT REFERENCES quest_series(id) ON DELETE CASCADE,
            period_key TEXT,
            is_checklist BOOLEAN NOT NULL DEFAULT 0,
            checklist_base TEXT
        );
        CREATE TABLE checklist_activity (
            id TEXT PRIMARY KEY NOT NULL,
            quest_id TEXT NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL,
            created_at TEXT NOT NULL,
            origin_device_id TEXT
        );
        CREATE INDEX idx_checklist_activity_quest_id ON checklist_activity (quest_id);
        ",
    )
    .map_err(|e| e.to_string())
}

fn validate_backup_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    let rows = diesel::sql_query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('spaces', 'quest_series', 'quests')",
    )
    .load::<TableNameRow>(conn)
    .map_err(|e| format!("failed to inspect backup schema: {e}"))?;
    let names: HashSet<String> = rows.into_iter().map(|row| row.name).collect();
    for required in ["spaces", "quest_series", "quests"] {
        if !names.contains(required) {
            return Err(format!(
                "backup database is missing required table {required}"
            ));
        }
    }
    Ok(())
}

fn migrate_backup_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    ensure_backup_column(
        conn,
        "quest_series",
        "is_checklist",
        "ALTER TABLE quest_series ADD COLUMN is_checklist BOOLEAN NOT NULL DEFAULT 0",
    )?;
    ensure_backup_column(
        conn,
        "quests",
        "is_checklist",
        "ALTER TABLE quests ADD COLUMN is_checklist BOOLEAN NOT NULL DEFAULT 0",
    )?;
    ensure_backup_column(
        conn,
        "quests",
        "checklist_base",
        "ALTER TABLE quests ADD COLUMN checklist_base TEXT",
    )?;
    ensure_checklist_activity_table(conn)
}

fn ensure_checklist_activity_table(conn: &mut SqliteConnection) -> Result<(), String> {
    conn.batch_execute(
        "
        CREATE TABLE IF NOT EXISTS checklist_activity (
            id TEXT PRIMARY KEY NOT NULL,
            quest_id TEXT NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL,
            created_at TEXT NOT NULL,
            origin_device_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_checklist_activity_quest_id ON checklist_activity (quest_id);
        ",
    )
    .map_err(|e| format!("failed to migrate backup schema for checklist_activity: {e}"))
}

fn ensure_backup_column(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
    migration: &str,
) -> Result<(), String> {
    let rows = diesel::sql_query(format!("SELECT name FROM pragma_table_info('{table}')"))
        .load::<ColumnNameRow>(conn)
        .map_err(|e| format!("failed to inspect backup schema columns for {table}: {e}"))?;
    if rows.iter().any(|row| row.name == column) {
        return Ok(());
    }
    conn.batch_execute(migration)
        .map_err(|e| format!("failed to migrate backup schema for {table}.{column}: {e}"))
}

fn validate_manifest(manifest: &BackupManifest) -> Result<(), String> {
    if manifest.format != BACKUP_FORMAT {
        return Err("unsupported backup format".to_string());
    }
    if manifest.version != BACKUP_VERSION {
        return Err("unsupported backup version".to_string());
    }
    Ok(())
}

fn write_zip(output_path: &Path, manifest: &BackupManifest, db_path: &Path) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create backup directory: {e}"))?;
    }

    let file =
        File::create(output_path).map_err(|e| format!("failed to create backup zip: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file(MANIFEST_NAME, options)
        .map_err(|e| e.to_string())?;
    let manifest_text = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    zip.write_all(&manifest_text).map_err(|e| e.to_string())?;

    zip.start_file(BACKUP_DB_NAME, options)
        .map_err(|e| e.to_string())?;
    let mut db_file = File::open(db_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut db_file, &mut zip).map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_backup(path: &Path) -> Result<ExtractedBackup, String> {
    let file = File::open(path).map_err(|e| format!("failed to open backup: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "backup must be a zip file".to_string())?;
    if archive.len() != 2 {
        return Err(
            "backup zip must contain exactly manifest.json and fini-backup.sqlite".to_string(),
        );
    }
    let mut names = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|e| e.to_string())?;
        names.push(file.name().to_string());
    }
    names.sort();
    if names != [BACKUP_DB_NAME.to_string(), MANIFEST_NAME.to_string()] {
        return Err(
            "backup zip must contain exactly manifest.json and fini-backup.sqlite".to_string(),
        );
    }

    let mut manifest_file = archive.by_name(MANIFEST_NAME).map_err(|e| e.to_string())?;
    let mut manifest_text = String::new();
    manifest_file
        .read_to_string(&mut manifest_text)
        .map_err(|e| e.to_string())?;
    let manifest: BackupManifest =
        serde_json::from_str(&manifest_text).map_err(|e| format!("invalid manifest.json: {e}"))?;
    drop(manifest_file);

    let temp_dir = create_temp_dir("import")?;
    let db_path = temp_dir.join(BACKUP_DB_NAME);
    let extraction: Result<(), String> = (|| {
        let mut db_file = archive.by_name(BACKUP_DB_NAME).map_err(|e| e.to_string())?;
        let mut out = File::create(&db_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut db_file, &mut out).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if extraction.is_err() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    extraction?;

    Ok(ExtractedBackup {
        temp_dir,
        db_path,
        manifest,
    })
}

fn load_backup_spaces(conn: &mut SqliteConnection) -> Result<Vec<Space>, String> {
    spaces::table
        .select(Space::as_select())
        .order(spaces::item_order.asc())
        .load(conn)
        .map_err(|e| e.to_string())
}

fn load_backup_series(conn: &mut SqliteConnection) -> Result<Vec<QuestSeries>, String> {
    quest_series::table
        .select(QuestSeries::as_select())
        .load(conn)
        .map_err(|e| e.to_string())
}

fn load_backup_quests(conn: &mut SqliteConnection) -> Result<Vec<Quest>, String> {
    quests::table
        .select(Quest::as_select())
        .load(conn)
        .map_err(|e| e.to_string())
}

fn load_backup_checklist_activity(
    conn: &mut SqliteConnection,
) -> Result<Vec<ChecklistActivity>, String> {
    checklist_activity::table
        .select(ChecklistActivity::as_select())
        .load(conn)
        .map_err(|e| e.to_string())
}

fn load_checklist_activity_for_quest(
    conn: &mut SqliteConnection,
    quest_id: &str,
) -> Result<Vec<ChecklistActivity>, String> {
    checklist_activity::table
        .filter(checklist_activity::quest_id.eq(quest_id))
        .order(checklist_activity::id.asc())
        .select(ChecklistActivity::as_select())
        .load(conn)
        .map_err(|e| e.to_string())
}

fn load_backup_checklist_activity_for_quest(
    conn: &mut SqliteConnection,
    quest_id: &str,
) -> Result<Vec<ChecklistActivity>, String> {
    load_checklist_activity_for_quest(conn, quest_id)
}

fn required_space_mappings(
    conn: &mut SqliteConnection,
    backup_spaces: &[Space],
    mappings: &[BackupSpaceMappingInput],
) -> Result<Vec<BackupSpaceMappingRequest>, String> {
    let mapped: HashSet<&str> = mappings
        .iter()
        .map(|m| m.backup_space_id.as_str())
        .collect();
    let mut required = Vec::new();
    for space in backup_spaces {
        if is_builtin_space(&space.id)
            || local_space_exists(conn, &space.id).map_err(|e| e.to_string())?
        {
            continue;
        }
        if mapped.contains(space.id.as_str()) {
            continue;
        }
        required.push(BackupSpaceMappingRequest {
            backup_space_id: space.id.clone(),
            backup_space_name: space.name.clone(),
        });
    }
    Ok(required)
}

fn resolve_space_map(
    conn: &mut SqliteConnection,
    backup_spaces: &[Space],
    mappings: &[BackupSpaceMappingInput],
) -> Result<HashMap<String, String>, String> {
    let mapping_by_backup: HashMap<&str, &BackupSpaceMappingInput> = mappings
        .iter()
        .map(|mapping| (mapping.backup_space_id.as_str(), mapping))
        .collect();
    let mut result = HashMap::new();
    for space in backup_spaces {
        if is_builtin_space(&space.id)
            || local_space_exists(conn, &space.id).map_err(|e| e.to_string())?
        {
            result.insert(space.id.clone(), space.id.clone());
            continue;
        }
        let Some(mapping) = mapping_by_backup.get(space.id.as_str()) else {
            continue;
        };
        match mapping.mode.as_str() {
            "create_new" => {
                result.insert(space.id.clone(), space.id.clone());
            }
            "use_existing" => {
                let local_space_id = mapping
                    .local_space_id
                    .as_deref()
                    .ok_or_else(|| "existing local space id is required".to_string())?;
                if !local_space_exists(conn, local_space_id).map_err(|e| e.to_string())? {
                    return Err("selected local space does not exist".to_string());
                }
                result.insert(space.id.clone(), local_space_id.to_string());
            }
            _ => return Err("invalid backup space mapping mode".to_string()),
        }
    }
    Ok(result)
}

fn collect_conflicts(
    conn: &mut SqliteConnection,
    backup_conn: &mut SqliteConnection,
    space_map: &HashMap<String, String>,
) -> Result<Vec<BackupConflict>, String> {
    let mut conflicts = Vec::new();

    for backup in load_backup_series(backup_conn)? {
        let mapped = mapped_series(&backup, space_map);
        if let Some(local) = quest_series::table
            .find(&mapped.id)
            .select(QuestSeries::as_select())
            .first::<QuestSeries>(conn)
            .optional()
            .map_err(|e| e.to_string())?
        {
            if series_value(&local)? != series_value(&mapped)? {
                conflicts.push(BackupConflict {
                    entity_type: "quest_series".to_string(),
                    id: mapped.id.clone(),
                    title: mapped.title.clone(),
                    local_summary: local.title.clone(),
                    backup_summary: mapped.title.clone(),
                    local: series_value(&local)?,
                    backup: series_value(&mapped)?,
                });
            }
        }
    }

    for backup in load_backup_quests(backup_conn)? {
        let mapped = mapped_quest(&backup, space_map);
        let backup_checklist_activity =
            load_backup_checklist_activity_for_quest(backup_conn, &backup.id)?;
        if let Some(local) = quests::table
            .find(&mapped.id)
            .select(Quest::as_select())
            .first::<Quest>(conn)
            .optional()
            .map_err(|e| e.to_string())?
        {
            let local_checklist_activity = load_checklist_activity_for_quest(conn, &mapped.id)?;
            if quest_value(&local)? != quest_value(&mapped)? {
                conflicts.push(BackupConflict {
                    entity_type: "quest".to_string(),
                    id: mapped.id.clone(),
                    title: mapped.title.clone(),
                    local_summary: format!("{} ({})", local.title, local.status),
                    backup_summary: format!("{} ({})", mapped.title, mapped.status),
                    local: quest_value(&local)?,
                    backup: quest_value(&mapped)?,
                });
            } else if checklist_activity_value(&local_checklist_activity)?
                != checklist_activity_value(&backup_checklist_activity)?
            {
                conflicts.push(BackupConflict {
                    entity_type: "quest".to_string(),
                    id: mapped.id.clone(),
                    title: mapped.title.clone(),
                    local_summary: format!(
                        "{} checklist activity row(s)",
                        local_checklist_activity.len()
                    ),
                    backup_summary: format!(
                        "{} checklist activity row(s)",
                        backup_checklist_activity.len()
                    ),
                    local: checklist_activity_value(&local_checklist_activity)?,
                    backup: checklist_activity_value(&backup_checklist_activity)?,
                });
            }
        }
    }

    Ok(conflicts)
}

fn insert_space(
    conn: &mut SqliteConnection,
    space: &Space,
) -> Result<usize, diesel::result::Error> {
    diesel::insert_into(spaces::table)
        .values((
            spaces::id.eq(&space.id),
            spaces::name.eq(&space.name),
            spaces::item_order.eq(space.item_order),
            spaces::created_at.eq(&space.created_at),
        ))
        .execute(conn)
}

fn insert_series(
    conn: &mut SqliteConnection,
    series: &QuestSeries,
) -> Result<usize, diesel::result::Error> {
    diesel::insert_into(quest_series::table)
        .values((
            quest_series::id.eq(&series.id),
            quest_series::space_id.eq(&series.space_id),
            quest_series::title.eq(&series.title),
            quest_series::description.eq(&series.description),
            quest_series::repeat_rule.eq(&series.repeat_rule),
            quest_series::priority.eq(series.priority),
            quest_series::energy.eq(&series.energy),
            quest_series::active.eq(series.active),
            quest_series::created_at.eq(&series.created_at),
            quest_series::updated_at.eq(&series.updated_at),
            quest_series::is_checklist.eq(series.is_checklist),
        ))
        .execute(conn)
}

fn insert_quest(
    conn: &mut SqliteConnection,
    quest: &Quest,
) -> Result<usize, diesel::result::Error> {
    diesel::insert_into(quests::table)
        .values((
            quests::id.eq(&quest.id),
            quests::space_id.eq(&quest.space_id),
            quests::title.eq(&quest.title),
            quests::description.eq(&quest.description),
            quests::status.eq(&quest.status),
            quests::energy.eq(&quest.energy),
            quests::priority.eq(quest.priority),
            quests::pinned.eq(quest.pinned),
            quests::due.eq(&quest.due),
            quests::due_time.eq(&quest.due_time),
            quests::repeat_rule.eq(&quest.repeat_rule),
            quests::completed_at.eq(&quest.completed_at),
            quests::order_rank.eq(quest.order_rank),
            quests::focus_enter_count.eq(quest.focus_enter_count),
            quests::created_at.eq(&quest.created_at),
            quests::updated_at.eq(&quest.updated_at),
            quests::series_id.eq(&quest.series_id),
            quests::period_key.eq(&quest.period_key),
            quests::is_checklist.eq(quest.is_checklist),
            quests::checklist_base.eq(&quest.checklist_base),
        ))
        .execute(conn)
}

fn insert_checklist_activity(
    conn: &mut SqliteConnection,
    activity: &ChecklistActivity,
) -> Result<usize, diesel::result::Error> {
    diesel::insert_into(checklist_activity::table)
        .values((
            checklist_activity::id.eq(&activity.id),
            checklist_activity::quest_id.eq(&activity.quest_id),
            checklist_activity::kind.eq(&activity.kind),
            checklist_activity::detail.eq(&activity.detail),
            checklist_activity::created_at.eq(&activity.created_at),
            checklist_activity::origin_device_id.eq(&activity.origin_device_id),
        ))
        .execute(conn)
}

fn insert_checklist_activity_for_import(
    conn: &mut SqliteConnection,
    activity: &ChecklistActivity,
) -> Result<(), diesel::result::Error> {
    diesel::insert_or_ignore_into(checklist_activity::table)
        .values((
            checklist_activity::id.eq(&activity.id),
            checklist_activity::quest_id.eq(&activity.quest_id),
            checklist_activity::kind.eq(&activity.kind),
            checklist_activity::detail.eq(&activity.detail),
            checklist_activity::created_at.eq(&activity.created_at),
            checklist_activity::origin_device_id.eq(&activity.origin_device_id),
        ))
        .execute(conn)?;
    Ok(())
}

fn delete_checklist_activity_for_quest(
    conn: &mut SqliteConnection,
    quest_id: &str,
) -> Result<usize, diesel::result::Error> {
    diesel::delete(checklist_activity::table.filter(checklist_activity::quest_id.eq(quest_id)))
        .execute(conn)
}

fn upsert_series_for_import(
    conn: &mut SqliteConnection,
    series: &QuestSeries,
    action: ConflictAction,
) -> Result<(), diesel::result::Error> {
    if matches!(action, ConflictAction::KeepLocal) {
        return Ok(());
    }
    let exists = quest_series::table
        .find(&series.id)
        .select(quest_series::id)
        .first::<String>(conn)
        .optional()?
        .is_some();
    if exists {
        diesel::update(quest_series::table.find(&series.id))
            .set((
                quest_series::space_id.eq(&series.space_id),
                quest_series::title.eq(&series.title),
                quest_series::description.eq(&series.description),
                quest_series::repeat_rule.eq(&series.repeat_rule),
                quest_series::priority.eq(series.priority),
                quest_series::energy.eq(&series.energy),
                quest_series::active.eq(series.active),
                quest_series::created_at.eq(&series.created_at),
                quest_series::updated_at.eq(&series.updated_at),
                quest_series::is_checklist.eq(series.is_checklist),
            ))
            .execute(conn)?;
    } else {
        insert_series(conn, series)?;
    }
    Ok(())
}

fn upsert_quest_for_import(
    conn: &mut SqliteConnection,
    quest: &Quest,
    action: ConflictAction,
) -> Result<(), diesel::result::Error> {
    if matches!(action, ConflictAction::KeepLocal) {
        return Ok(());
    }
    let exists = quests::table
        .find(&quest.id)
        .select(quests::id)
        .first::<String>(conn)
        .optional()?
        .is_some();
    if exists {
        // checklist_base is deliberately left out of this SET list: it's device-local per-item
        // sync convergence bookkeeping, never part of a quest's own data (see its doc comment —
        // "never included in sync payloads"). It's also excluded from quest_value()'s conflict
        // comparison for the same reason, so a quest that round-trips through backup/import with
        // no other changes never appears as a conflict at all — if this UPDATE still overwrote
        // checklist_base from the backup, a stale backup could silently regress convergence
        // bookkeeping for a quest that was never flagged as conflicting, causing the next sync
        // merge to misjudge which side already agreed on what.
        diesel::update(quests::table.find(&quest.id))
            .set((
                quests::space_id.eq(&quest.space_id),
                quests::title.eq(&quest.title),
                quests::description.eq(&quest.description),
                quests::status.eq(&quest.status),
                quests::energy.eq(&quest.energy),
                quests::priority.eq(quest.priority),
                quests::pinned.eq(quest.pinned),
                quests::due.eq(&quest.due),
                quests::due_time.eq(&quest.due_time),
                quests::repeat_rule.eq(&quest.repeat_rule),
                quests::completed_at.eq(&quest.completed_at),
                quests::order_rank.eq(quest.order_rank),
                quests::focus_enter_count.eq(quest.focus_enter_count),
                quests::created_at.eq(&quest.created_at),
                quests::updated_at.eq(&quest.updated_at),
                quests::series_id.eq(&quest.series_id),
                quests::period_key.eq(&quest.period_key),
                quests::is_checklist.eq(quest.is_checklist),
            ))
            .execute(conn)?;
    } else {
        insert_quest(conn, quest)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ConflictAction {
    KeepLocal,
    UseBackup,
}

fn conflict_action(
    resolution_map: &HashMap<(String, String), String>,
    entity_type: &str,
    id: &str,
) -> ConflictAction {
    match resolution_map
        .get(&(entity_type.to_string(), id.to_string()))
        .map(String::as_str)
    {
        Some("local") => ConflictAction::KeepLocal,
        _ => ConflictAction::UseBackup,
    }
}

fn local_space_exists(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<bool, diesel::result::Error> {
    spaces::table
        .find(id)
        .select(spaces::id)
        .first::<String>(conn)
        .optional()
        .map(|row| row.is_some())
}

fn mapped_series(series: &QuestSeries, space_map: &HashMap<String, String>) -> QuestSeries {
    QuestSeries {
        id: series.id.clone(),
        space_id: space_map
            .get(&series.space_id)
            .cloned()
            .unwrap_or_else(|| series.space_id.clone()),
        title: series.title.clone(),
        description: series.description.clone(),
        repeat_rule: series.repeat_rule.clone(),
        priority: series.priority,
        energy: series.energy.clone(),
        active: series.active,
        created_at: series.created_at.clone(),
        updated_at: series.updated_at.clone(),
        is_checklist: series.is_checklist,
    }
}

fn mapped_quest(quest: &Quest, space_map: &HashMap<String, String>) -> Quest {
    Quest {
        id: quest.id.clone(),
        space_id: space_map
            .get(&quest.space_id)
            .cloned()
            .unwrap_or_else(|| quest.space_id.clone()),
        title: quest.title.clone(),
        description: quest.description.clone(),
        status: quest.status.clone(),
        energy: quest.energy.clone(),
        priority: quest.priority,
        pinned: quest.pinned,
        due: quest.due.clone(),
        due_time: quest.due_time.clone(),
        repeat_rule: quest.repeat_rule.clone(),
        completed_at: quest.completed_at.clone(),
        order_rank: quest.order_rank,
        focus_enter_count: quest.focus_enter_count,
        created_at: quest.created_at.clone(),
        updated_at: quest.updated_at.clone(),
        series_id: quest.series_id.clone(),
        period_key: quest.period_key.clone(),
        is_checklist: quest.is_checklist,
        checklist_base: quest.checklist_base.clone(),
    }
}

fn series_value(series: &QuestSeries) -> Result<serde_json::Value, String> {
    serde_json::to_value(series).map_err(|e| e.to_string())
}

fn quest_value(quest: &Quest) -> Result<serde_json::Value, String> {
    serde_json::to_value(quest).map_err(|e| e.to_string())
}

fn checklist_activity_value(activity: &[ChecklistActivity]) -> Result<serde_json::Value, String> {
    serde_json::to_value(activity).map_err(|e| e.to_string())
}

fn is_builtin_space(id: &str) -> bool {
    BUILTIN_SPACE_IDS.contains(&id)
}

fn create_temp_dir(label: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("fini-backup-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create temp directory: {e}"))?;
    Ok(dir)
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "path must be valid UTF-8".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::{open_db_at_path, temp_db_path};
    use std::sync::Mutex;

    /// Every test in this module extracts backups through `create_temp_dir("import")`, which
    /// writes into the process-wide `fini-backup-import-*` namespace under the OS temp dir.
    /// Several tests assert on that namespace's before/after contents to prove cleanup happened;
    /// without serializing the module, one test's in-flight temp dir is indistinguishable from
    /// another concurrently-running test's leak, causing spurious failures.
    static TEMP_DIR_LOCK: Mutex<()> = Mutex::new(());

    fn lock_temp_dir_namespace() -> std::sync::MutexGuard<'static, ()> {
        TEMP_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn backup_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fini-{label}-{}.zip", Uuid::new_v4()))
    }

    fn seed_custom_space(conn: &mut SqliteConnection, id: &str, name: &str) {
        diesel::insert_into(spaces::table)
            .values((
                spaces::id.eq(id),
                spaces::name.eq(name),
                spaces::item_order.eq(10_i64),
            ))
            .execute(conn)
            .expect("insert custom space");
    }

    fn create_legacy_v2_backup_schema(conn: &mut SqliteConnection) {
        conn.batch_execute(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE spaces (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                item_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE TABLE quest_series (
                id TEXT PRIMARY KEY NOT NULL,
                space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE SET DEFAULT,
                title TEXT NOT NULL,
                description TEXT,
                repeat_rule TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 1,
                energy TEXT NOT NULL DEFAULT 'medium',
                active BOOLEAN NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE quests (
                id TEXT PRIMARY KEY NOT NULL,
                space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE SET DEFAULT,
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
                order_rank REAL NOT NULL DEFAULT 0,
                focus_enter_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                series_id TEXT REFERENCES quest_series(id) ON DELETE CASCADE,
                period_key TEXT
            );
            ",
        )
        .expect("create legacy backup schema");
    }

    fn write_legacy_v2_backup_archive(label: &str) -> (PathBuf, PathBuf) {
        let backup_database_path = temp_db_path(&format!("{label}-db"));
        let mut backup_conn =
            SqliteConnection::establish(path_str(&backup_database_path).expect("path"))
                .expect("create legacy backup database");
        create_legacy_v2_backup_schema(&mut backup_conn);
        backup_conn
            .batch_execute(
                "
                INSERT INTO spaces (id, name, item_order, created_at)
                VALUES ('1', 'Inbox', 0, '2026-01-01T00:00:00Z');
                INSERT INTO quest_series (
                    id, space_id, title, description, repeat_rule, priority, energy,
                    active, created_at, updated_at
                ) VALUES (
                    'legacy-series', '1', 'Legacy series', '- [ ] Template item',
                    '{\"type\":\"daily\"}', 2, 'medium', 1,
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
                );
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key
                ) VALUES (
                    'legacy-quest', '1', 'Legacy quest', '- [ ] Existing item', 'active',
                    'medium', 2, 0, NULL, NULL, NULL, NULL, 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                    'legacy-series', '2026-01-01'
                );
                ",
            )
            .expect("seed legacy backup database");
        drop(backup_conn);

        let archive_path = backup_path(label);
        let manifest = BackupManifest {
            format: BACKUP_FORMAT.to_string(),
            version: BACKUP_VERSION,
            app_version: "0.2.1".to_string(),
            exported_at: "2026-01-01T00:00:00Z".to_string(),
            domains: vec![
                "spaces".to_string(),
                "quest_series".to_string(),
                "quests".to_string(),
            ],
            spaces: vec![ManifestSpace {
                id: "1".to_string(),
                name: "Inbox".to_string(),
            }],
            counts: ManifestCounts {
                spaces: 1,
                quest_series: 1,
                quests: 1,
            },
        };
        write_zip(&archive_path, &manifest, &backup_database_path).expect("write legacy archive");
        (archive_path, backup_database_path)
    }

    #[test]
    fn dry_run_plan_reports_unresolved_conflicts_without_selecting_a_resolution() {
        let _guard = lock_temp_dir_namespace();
        let plan = BackupImportDryRunPlan::from_preflight(BackupImportPreflight {
            manifest: BackupManifest {
                format: BACKUP_FORMAT.to_string(),
                version: BACKUP_VERSION,
                app_version: "test".to_string(),
                exported_at: "2026-01-01T00:00:00Z".to_string(),
                domains: vec!["quests".to_string()],
                spaces: vec![],
                counts: ManifestCounts {
                    spaces: 0,
                    quest_series: 0,
                    quests: 1,
                },
            },
            required_space_mappings: vec![],
            conflicts: vec![BackupConflict {
                entity_type: "quest".to_string(),
                id: "conflicting-quest".to_string(),
                title: "Conflicting quest".to_string(),
                local_summary: "Local title (active)".to_string(),
                backup_summary: "Backup title (active)".to_string(),
                local: serde_json::json!({ "title": "Local title" }),
                backup: serde_json::json!({ "title": "Backup title" }),
            }],
        });

        let value = serde_json::to_value(plan).expect("serialize dry run plan");
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["ready_to_apply"], false);
        assert_eq!(value["no_apply_or_recovery_action_occurred"], true);
        assert_eq!(value["conflicts"][0]["id"], "conflicting-quest");
        assert!(
            value["conflicts"][0].get("resolution").is_none(),
            "a dry run must report conflicts without choosing a resolution"
        );
    }

    #[test]
    fn export_creates_zip_with_exact_required_files() {
        let _guard = lock_temp_dir_namespace();
        let db_path = temp_db_path("backup-export-exact-files");
        let mut conn = open_db_at_path(&db_path);
        let out_path = backup_path("backup-export-exact-files");

        export_backup(&mut conn, &out_path, &["1".to_string()]).expect("export backup");

        let file = File::open(&out_path).expect("open zip");
        let mut archive = zip::ZipArchive::new(file).expect("read zip");
        let mut names = Vec::new();
        for index in 0..archive.len() {
            names.push(
                archive
                    .by_index(index)
                    .expect("zip file")
                    .name()
                    .to_string(),
            );
        }
        names.sort();
        assert_eq!(
            names,
            [BACKUP_DB_NAME.to_string(), MANIFEST_NAME.to_string()]
        );

        let _ = fs::remove_file(out_path);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn import_preflight_requires_missing_custom_space_mapping() {
        let _guard = lock_temp_dir_namespace();
        let source_db_path = temp_db_path("backup-source-custom-space");
        let mut source = open_db_at_path(&source_db_path);
        seed_custom_space(&mut source, "space-custom", "Custom");
        let out_path = backup_path("backup-source-custom-space");
        export_backup(&mut source, &out_path, &["space-custom".to_string()]).expect("export");

        let target_db_path = temp_db_path("backup-target-custom-space");
        let mut target = open_db_at_path(&target_db_path);
        let preflight = preflight_import(&mut target, &out_path, &[]).expect("preflight");

        assert_eq!(preflight.required_space_mappings.len(), 1);
        assert_eq!(
            preflight.required_space_mappings[0].backup_space_id,
            "space-custom"
        );

        let _ = fs::remove_file(out_path);
        let _ = fs::remove_file(source_db_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn import_accepts_legacy_v2_backups_without_checklist_columns() {
        let _guard = lock_temp_dir_namespace();
        let (archive_path, backup_database_path) =
            write_legacy_v2_backup_archive("backup-legacy-v2-checklist-columns");

        let inspection = inspect_backup(&archive_path).expect("inspect legacy backup");
        assert_eq!(inspection.contents.counts.quest_series, 1);
        assert_eq!(inspection.contents.counts.quests, 1);

        let target_db_path = temp_db_path("backup-legacy-v2-target");
        let mut target = open_db_at_path(&target_db_path);
        let preflight = preflight_import(&mut target, &archive_path, &[]).expect("preflight");
        assert!(preflight.required_space_mappings.is_empty());
        assert!(preflight.conflicts.is_empty());

        apply_import(&mut target, &archive_path, &[], &[]).expect("import legacy backup");
        let imported_series: QuestSeries = quest_series::table
            .find("legacy-series")
            .select(QuestSeries::as_select())
            .first(&mut target)
            .expect("load imported series");
        let imported_quest: Quest = quests::table
            .find("legacy-quest")
            .select(Quest::as_select())
            .first(&mut target)
            .expect("load imported quest");
        assert!(!imported_series.is_checklist);
        assert!(!imported_quest.is_checklist);
        assert_eq!(imported_quest.checklist_base, None);

        let _ = fs::remove_file(archive_path);
        let _ = fs::remove_file(backup_database_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn backup_roundtrip_preserves_checklist_activity() {
        let _guard = lock_temp_dir_namespace();
        let source_db_path = temp_db_path("backup-source-checklist-activity");
        let mut source = open_db_at_path(&source_db_path);
        source
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'checklist-quest', '1', 'Packed list',
                    '- [x] Charger <!--k=item-1-->', 'completed', 'medium', 1, 0,
                    NULL, NULL, NULL, '2026-01-02T00:00:00Z', 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', NULL, NULL, 1,
                    '- [x] Charger <!--k=item-1-->'
                );
                INSERT INTO checklist_activity (
                    id, quest_id, kind, detail, created_at, origin_device_id
                ) VALUES (
                    'activity-completed', 'checklist-quest', 'completed_snapshot',
                    'Checklist at completion: 1/1 checked', '2026-01-02T00:00:00Z', NULL
                ), (
                    'activity-edit', 'checklist-quest', 'post_completion_edit',
                    'Renamed item to "USB-C charger"', '2026-01-03T00:00:00Z', 'peer-device'
                );
                "#,
            )
            .expect("seed source checklist activity");

        let out_path = backup_path("backup-checklist-activity");
        export_backup(&mut source, &out_path, &["1".to_string()]).expect("export");

        let target_db_path = temp_db_path("backup-target-checklist-activity");
        let mut target = open_db_at_path(&target_db_path);
        apply_import(&mut target, &out_path, &[], &[]).expect("import");

        let imported_activity = checklist_activity::table
            .filter(checklist_activity::quest_id.eq("checklist-quest"))
            .order(checklist_activity::created_at.asc())
            .select(ChecklistActivity::as_select())
            .load::<ChecklistActivity>(&mut target)
            .expect("load imported checklist activity");
        assert_eq!(imported_activity.len(), 2);
        assert_eq!(imported_activity[0].id, "activity-completed");
        assert_eq!(imported_activity[0].kind, "completed_snapshot");
        assert_eq!(imported_activity[1].id, "activity-edit");
        assert_eq!(imported_activity[1].kind, "post_completion_edit");
        assert_eq!(
            imported_activity[1].origin_device_id.as_deref(),
            Some("peer-device")
        );

        let _ = fs::remove_file(out_path);
        let _ = fs::remove_file(source_db_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn import_backup_resolution_replaces_local_checklist_activity() {
        let _guard = lock_temp_dir_namespace();
        let source_db_path = temp_db_path("backup-source-replace-checklist-activity");
        let mut source = open_db_at_path(&source_db_path);
        source
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'replace-activity-quest', '1', 'Backup title',
                    '- [x] Backup item <!--k=item-1-->', 'completed', 'medium', 1, 0,
                    NULL, NULL, NULL, '2026-01-02T00:00:00Z', 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', NULL, NULL, 1,
                    '- [x] Backup item <!--k=item-1-->'
                );
                INSERT INTO checklist_activity (
                    id, quest_id, kind, detail, created_at, origin_device_id
                ) VALUES (
                    'shared-activity', 'replace-activity-quest', 'completed_snapshot',
                    'Backup snapshot', '2026-01-02T00:00:00Z', 'backup-device'
                );
                "#,
            )
            .expect("seed backup checklist activity");

        let out_path = backup_path("backup-replace-checklist-activity");
        export_backup(&mut source, &out_path, &["1".to_string()]).expect("export");

        let target_db_path = temp_db_path("backup-target-replace-checklist-activity");
        let mut target = open_db_at_path(&target_db_path);
        target
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'replace-activity-quest', '1', 'Local title',
                    '- [x] Local item <!--k=item-1-->', 'completed', 'medium', 1, 0,
                    NULL, NULL, NULL, '2026-01-02T00:00:00Z', 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-03T00:00:00Z', NULL, NULL, 1,
                    '- [x] Local item <!--k=item-1-->'
                );
                INSERT INTO checklist_activity (
                    id, quest_id, kind, detail, created_at, origin_device_id
                ) VALUES (
                    'shared-activity', 'replace-activity-quest', 'completed_snapshot',
                    'Stale local snapshot', '2026-01-02T00:00:00Z', 'local-device'
                ), (
                    'local-only-activity', 'replace-activity-quest', 'post_completion_edit',
                    'Local-only edit', '2026-01-03T00:00:00Z', 'local-device'
                );
                "#,
            )
            .expect("seed target stale checklist activity");

        apply_import(
            &mut target,
            &out_path,
            &[],
            &[BackupConflictResolutionInput {
                entity_type: "quest".to_string(),
                id: "replace-activity-quest".to_string(),
                resolution: "backup".to_string(),
            }],
        )
        .expect("import backup version");

        let imported_activity = checklist_activity::table
            .filter(checklist_activity::quest_id.eq("replace-activity-quest"))
            .order(checklist_activity::created_at.asc())
            .select(ChecklistActivity::as_select())
            .load::<ChecklistActivity>(&mut target)
            .expect("load replaced checklist activity");
        assert_eq!(imported_activity.len(), 1);
        assert_eq!(imported_activity[0].id, "shared-activity");
        assert_eq!(imported_activity[0].detail, "Backup snapshot");
        assert_eq!(
            imported_activity[0].origin_device_id.as_deref(),
            Some("backup-device")
        );

        let _ = fs::remove_file(out_path);
        let _ = fs::remove_file(source_db_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn import_preserves_local_checklist_base_for_a_non_conflicting_quest() {
        let _guard = lock_temp_dir_namespace();
        let source_db_path = temp_db_path("backup-source-checklist-base");
        let mut source = open_db_at_path(&source_db_path);
        source
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'base-quest', '1', 'Pack bag',
                    '- [ ] headphones <!--k=a1-->', 'active', 'medium', 1, 0,
                    NULL, NULL, NULL, NULL, 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL, 1,
                    'stale-backup-base'
                );
                "#,
            )
            .expect("seed source quest");

        let out_path = backup_path("backup-checklist-base");
        export_backup(&mut source, &out_path, &["1".to_string()]).expect("export");

        let target_db_path = temp_db_path("backup-target-checklist-base");
        let mut target = open_db_at_path(&target_db_path);
        target
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'base-quest', '1', 'Pack bag',
                    '- [ ] headphones <!--k=a1-->', 'active', 'medium', 1, 0,
                    NULL, NULL, NULL, NULL, 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL, 1,
                    'current-local-base'
                );
                "#,
            )
            .expect("seed target quest with an identical quest but a different local base");

        let preflight = preflight_import(&mut target, &out_path, &[]).expect("preflight");
        assert!(
            preflight.conflicts.is_empty(),
            "checklist_base must not be part of conflict detection — it's device-local \
             bookkeeping, not quest data — got: {:?}",
            preflight.conflicts
        );

        apply_import(&mut target, &out_path, &[], &[]).expect("import with no conflicts to resolve");

        let imported: Quest = quests::table
            .find("base-quest")
            .select(Quest::as_select())
            .first(&mut target)
            .expect("load imported quest");
        assert_eq!(
            imported.checklist_base.as_deref(),
            Some("current-local-base"),
            "a non-conflicting quest's import must not overwrite the local checklist_base with a \
             stale backup value — that would desync the next per-item sync merge's convergence \
             bookkeeping"
        );

        let _ = fs::remove_file(out_path);
        let _ = fs::remove_file(source_db_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn preflight_reports_checklist_activity_conflicts_before_import() {
        let _guard = lock_temp_dir_namespace();
        let source_db_path = temp_db_path("backup-source-activity-conflict");
        let mut source = open_db_at_path(&source_db_path);
        source
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'activity-conflict-quest', '1', 'Packed list',
                    '- [x] Charger <!--k=item-1-->', 'completed', 'medium', 1, 0,
                    NULL, NULL, NULL, '2026-01-02T00:00:00Z', 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', NULL, NULL, 1,
                    '- [x] Charger <!--k=item-1-->'
                );
                INSERT INTO checklist_activity (
                    id, quest_id, kind, detail, created_at, origin_device_id
                ) VALUES (
                    'backup-activity', 'activity-conflict-quest', 'completed_snapshot',
                    'Backup snapshot', '2026-01-02T00:00:00Z', 'backup-device'
                );
                "#,
            )
            .expect("seed backup checklist activity");

        let out_path = backup_path("backup-activity-conflict");
        export_backup(&mut source, &out_path, &["1".to_string()]).expect("export");

        let target_db_path = temp_db_path("backup-target-activity-conflict");
        let mut target = open_db_at_path(&target_db_path);
        target
            .batch_execute(
                r#"
                INSERT INTO quests (
                    id, space_id, title, description, status, energy, priority, pinned,
                    due, due_time, repeat_rule, completed_at, order_rank, focus_enter_count,
                    created_at, updated_at, series_id, period_key, is_checklist, checklist_base
                ) VALUES (
                    'activity-conflict-quest', '1', 'Packed list',
                    '- [x] Charger <!--k=item-1-->', 'completed', 'medium', 1, 0,
                    NULL, NULL, NULL, '2026-01-02T00:00:00Z', 0, 0,
                    '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', NULL, NULL, 1,
                    '- [x] Charger <!--k=item-1-->'
                );
                INSERT INTO checklist_activity (
                    id, quest_id, kind, detail, created_at, origin_device_id
                ) VALUES (
                    'local-activity', 'activity-conflict-quest', 'completed_snapshot',
                    'Local snapshot', '2026-01-02T00:00:00Z', 'local-device'
                );
                "#,
            )
            .expect("seed local checklist activity");

        let preflight = preflight_import(&mut target, &out_path, &[]).expect("preflight");
        assert_eq!(preflight.conflicts.len(), 1);
        assert_eq!(preflight.conflicts[0].entity_type, "quest");
        assert_eq!(preflight.conflicts[0].id, "activity-conflict-quest");
        assert_eq!(preflight.conflicts[0].local[0]["id"], "local-activity");
        assert_eq!(preflight.conflicts[0].backup[0]["id"], "backup-activity");

        let error = apply_import(&mut target, &out_path, &[], &[])
            .expect_err("activity conflict must require explicit resolution");
        assert_eq!(error, "resolve every backup conflict before import");

        apply_import(
            &mut target,
            &out_path,
            &[],
            &[BackupConflictResolutionInput {
                entity_type: "quest".to_string(),
                id: "activity-conflict-quest".to_string(),
                resolution: "local".to_string(),
            }],
        )
        .expect("keep local activity");

        let retained_activity = checklist_activity::table
            .filter(checklist_activity::quest_id.eq("activity-conflict-quest"))
            .select(ChecklistActivity::as_select())
            .load::<ChecklistActivity>(&mut target)
            .expect("load retained checklist activity");
        assert_eq!(retained_activity.len(), 1);
        assert_eq!(retained_activity[0].id, "local-activity");
        assert_eq!(retained_activity[0].detail, "Local snapshot");

        let _ = fs::remove_file(out_path);
        let _ = fs::remove_file(source_db_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn import_preflight_removes_temp_dir_when_schema_validation_fails() {
        let _guard = lock_temp_dir_namespace();
        let backup_database_path = temp_db_path("backup-invalid-schema");
        let backup_conn =
            SqliteConnection::establish(path_str(&backup_database_path).expect("path"))
                .expect("create empty backup database");
        drop(backup_conn);
        let backup_path = backup_path("backup-invalid-schema");
        let manifest = BackupManifest {
            format: BACKUP_FORMAT.to_string(),
            version: BACKUP_VERSION,
            app_version: "test".to_string(),
            exported_at: "2026-01-01T00:00:00Z".to_string(),
            domains: vec![],
            spaces: vec![],
            counts: ManifestCounts {
                spaces: 0,
                quest_series: 0,
                quests: 0,
            },
        };
        write_zip(&backup_path, &manifest, &backup_database_path).expect("write backup archive");

        let target_db_path = temp_db_path("backup-invalid-schema-target");
        let mut target = open_db_at_path(&target_db_path);
        let before: HashSet<PathBuf> = fs::read_dir(std::env::temp_dir())
            .expect("read temp directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("fini-backup-import-"))
            })
            .collect();

        let error = preflight_import(&mut target, &backup_path, &[])
            .expect_err("an empty database must fail schema validation");
        assert_eq!(error, "backup database is missing required table spaces");

        let after: HashSet<PathBuf> = fs::read_dir(std::env::temp_dir())
            .expect("read temp directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("fini-backup-import-"))
            })
            .collect();
        assert!(
            after.is_subset(&before),
            "failed preflight must not leave a new import temp directory"
        );

        let _ = fs::remove_file(backup_path);
        let _ = fs::remove_file(backup_database_path);
        let _ = fs::remove_file(target_db_path);
    }

    #[test]
    fn extract_backup_removes_temp_dir_when_database_copy_fails() {
        let _guard = lock_temp_dir_namespace();
        let backup_path = backup_path("backup-corrupt-database");
        let manifest = BackupManifest {
            format: BACKUP_FORMAT.to_string(),
            version: BACKUP_VERSION,
            app_version: "test".to_string(),
            exported_at: "2026-01-01T00:00:00Z".to_string(),
            domains: vec![],
            spaces: vec![],
            counts: ManifestCounts {
                spaces: 0,
                quest_series: 0,
                quests: 0,
            },
        };
        let file = File::create(&backup_path).expect("create corrupt backup archive");
        let mut archive = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file(MANIFEST_NAME, options)
            .expect("add manifest to archive");
        archive
            .write_all(&serde_json::to_vec(&manifest).expect("serialize manifest"))
            .expect("write manifest to archive");
        archive
            .start_file(BACKUP_DB_NAME, options)
            .expect("add database to archive");
        archive
            .write_all(b"database bytes that will fail CRC validation")
            .expect("write database to archive");
        archive.finish().expect("finish archive");

        let mut bytes = fs::read(&backup_path).expect("read backup archive");
        let database_offset = bytes
            .windows(b"database bytes that will fail CRC validation".len())
            .position(|window| window == b"database bytes that will fail CRC validation")
            .expect("find stored database bytes");
        bytes[database_offset] ^= 1;
        fs::write(&backup_path, bytes).expect("corrupt database bytes");

        let before: HashSet<PathBuf> = fs::read_dir(std::env::temp_dir())
            .expect("read temp directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("fini-backup-import-"))
            })
            .collect();

        assert!(
            extract_backup(&backup_path).is_err(),
            "copy must fail on bad CRC"
        );

        let after: HashSet<PathBuf> = fs::read_dir(std::env::temp_dir())
            .expect("read temp directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("fini-backup-import-"))
            })
            .collect();
        assert!(
            after.is_subset(&before),
            "failed extraction must not leave a new import temp directory"
        );

        let _ = fs::remove_file(backup_path);
    }
}
