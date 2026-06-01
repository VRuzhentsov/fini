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
use tauri::State;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::models::{Quest, QuestSeries, Space};
use crate::schema::{quest_series, quests, spaces};
use crate::services::db::utc_now;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;

const MANIFEST_NAME: &str = "manifest.json";
const BACKUP_DB_NAME: &str = "fini-backup.sqlite";
const BACKUP_FORMAT: &str = "fini-backup";
const BACKUP_VERSION: u32 = 1;
const BUILTIN_SPACE_IDS: [&str; 3] = ["1", "2", "3"];

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub domains: Vec<String>,
    pub spaces: Vec<ManifestSpace>,
    pub counts: ManifestCounts,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestSpace {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
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
pub struct BackupImportPreflight {
    pub manifest: BackupManifest,
    pub required_space_mappings: Vec<BackupSpaceMappingRequest>,
    pub conflicts: Vec<BackupConflict>,
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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn backup_export(
    state: State<AppDbConnection>,
    path: String,
    space_ids: Vec<String>,
) -> Result<BackupExportResult, String> {
    let mut conn = state.inner().0.lock().unwrap();
    export_backup(&mut conn, Path::new(&path), &space_ids)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn backup_preflight_import(
    state: State<AppDbConnection>,
    path: String,
    mappings: Vec<BackupSpaceMappingInput>,
) -> Result<BackupImportPreflight, String> {
    let mut conn = state.inner().0.lock().unwrap();
    preflight_import(&mut conn, Path::new(&path), &mappings)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn backup_apply_import(
    state: State<AppDbConnection>,
    path: String,
    mappings: Vec<BackupSpaceMappingInput>,
    resolutions: Vec<BackupConflictResolutionInput>,
) -> Result<BackupImportResult, String> {
    let mut conn = state.inner().0.lock().unwrap();
    apply_import(&mut conn, Path::new(&path), &mappings, &resolutions)
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
    let extracted = extract_backup(path)?;
    let mut backup_conn = SqliteConnection::establish(path_str(&extracted.db_path)?)
        .map_err(|e| format!("failed to open backup database: {e}"))?;
    validate_backup_schema(&mut backup_conn)?;
    validate_manifest(&extracted.manifest)?;

    let backup_spaces = load_backup_spaces(&mut backup_conn)?;
    let space_map = resolve_space_map(conn, &backup_spaces, mappings)?;
    let required_space_mappings = required_space_mappings(conn, &backup_spaces, mappings)?;
    let conflicts = collect_conflicts(conn, &mut backup_conn, &space_map)?;
    let _ = fs::remove_dir_all(&extracted.temp_dir);

    Ok(BackupImportPreflight {
        manifest: extracted.manifest,
        required_space_mappings,
        conflicts,
    })
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

    let extracted = extract_backup(path)?;
    let mut backup_conn = SqliteConnection::establish(path_str(&extracted.db_path)?)
        .map_err(|e| format!("failed to open backup database: {e}"))?;
    validate_backup_schema(&mut backup_conn)?;
    validate_manifest(&extracted.manifest)?;

    let backup_spaces = load_backup_spaces(&mut backup_conn)?;
    let backup_series = load_backup_series(&mut backup_conn)?;
    let backup_quests = load_backup_quests(&mut backup_conn)?;
    let space_map = resolve_space_map(conn, &backup_spaces, mappings)?;
    let backup_space_by_id: HashMap<&str, &Space> = backup_spaces
        .iter()
        .map(|space| (space.id.as_str(), space))
        .collect();

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

        Ok(())
    })
    .map_err(|e| e.to_string())?;

    let _ = fs::remove_dir_all(&extracted.temp_dir);
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
    ensure_backup_quest_columns(conn)?;
    Ok(())
}

fn ensure_backup_quest_columns(conn: &mut SqliteConnection) -> Result<(), String> {
    let rows = diesel::sql_query("PRAGMA table_info(quests)")
        .load::<TableNameRow>(conn)
        .map_err(|e| format!("failed to inspect backup quest columns: {e}"))?;
    let columns: HashSet<String> = rows.into_iter().map(|row| row.name).collect();

    if !columns.contains("focus_enter_count") {
        conn.batch_execute(
            "ALTER TABLE quests ADD COLUMN focus_enter_count INTEGER NOT NULL DEFAULT 0",
        )
        .map_err(|e| format!("failed to upgrade backup quest schema: {e}"))?;
    }

    Ok(())
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
    let mut db_file = archive.by_name(BACKUP_DB_NAME).map_err(|e| e.to_string())?;
    let mut out = File::create(&db_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut db_file, &mut out).map_err(|e| e.to_string())?;

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
        if let Some(local) = quests::table
            .find(&mapped.id)
            .select(Quest::as_select())
            .first::<Quest>(conn)
            .optional()
            .map_err(|e| e.to_string())?
        {
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
        ))
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
    }
}

fn series_value(series: &QuestSeries) -> Result<serde_json::Value, String> {
    serde_json::to_value(series).map_err(|e| e.to_string())
}

fn quest_value(quest: &Quest) -> Result<serde_json::Value, String> {
    serde_json::to_value(quest).map_err(|e| e.to_string())
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

    #[test]
    fn export_creates_zip_with_exact_required_files() {
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
}
