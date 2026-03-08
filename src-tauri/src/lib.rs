mod voice;
mod model_download;
use voice::{model_downloaded, start_recognition, stop_recognition, VoiceState};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Manager, State};

pub struct DbState(pub Mutex<Connection>);

#[derive(Serialize, Deserialize)]
pub struct Quest {
    pub id: i64,
    pub title: String,
    pub status: String,
    pub energy_required: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
pub struct QuestStep {
    pub id: i64,
    pub quest_id: i64,
    pub body: String,
    pub done: bool,
    pub step_order: i64,
}

#[derive(Serialize, Deserialize)]
pub struct QuestWithSteps {
    #[serde(flatten)]
    pub quest: Quest,
    pub steps: Vec<QuestStep>,
}

fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY
        );",
    )?;

    let version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?;

    if version < 1 {
        conn.execute_batch(include_str!("../migrations/001_init.sql"))?;
        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (1)",
            [],
        )?;
    }

    Ok(())
}

fn open_db(app: &tauri::AppHandle) -> rusqlite::Result<Connection> {
    let data_dir = app
        .path()
        .app_data_dir()
        .expect("failed to resolve app data dir");
    std::fs::create_dir_all(&data_dir).expect("failed to create app data dir");
    let conn = Connection::open(data_dir.join("fini.db"))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    run_migrations(&conn)?;
    Ok(conn)
}

// --- Quest commands ---

#[tauri::command]
fn create_quest(
    state: State<DbState>,
    title: String,
    energy_required: Option<i64>,
) -> Result<Quest, String> {
    let conn = state.inner().0.lock().unwrap();
    conn.execute(
        "INSERT INTO quests (title, energy_required) VALUES (?1, ?2)",
        params![title, energy_required],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    conn.query_row(
        "SELECT id, title, status, energy_required, created_at, updated_at FROM quests WHERE id = ?1",
        params![id],
        |r| Ok(Quest {
            id: r.get(0)?,
            title: r.get(1)?,
            status: r.get(2)?,
            energy_required: r.get(3)?,
            created_at: r.get(4)?,
            updated_at: r.get(5)?,
        }),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_quests(state: State<DbState>) -> Result<Vec<Quest>, String> {
    let conn = state.inner().0.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, title, status, energy_required, created_at, updated_at
             FROM quests ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let quests = stmt
        .query_map([], |r| Ok(Quest {
            id: r.get(0)?,
            title: r.get(1)?,
            status: r.get(2)?,
            energy_required: r.get(3)?,
            created_at: r.get(4)?,
            updated_at: r.get(5)?,
        }))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(quests)
}

#[tauri::command]
fn get_active_quest(state: State<DbState>) -> Result<Option<QuestWithSteps>, String> {
    let conn = state.inner().0.lock().unwrap();
    let quest_opt = conn
        .query_row(
            "SELECT id, title, status, energy_required, created_at, updated_at
             FROM quests WHERE status = 'active' ORDER BY created_at DESC LIMIT 1",
            [],
            |r| Ok(Quest {
                id: r.get(0)?,
                title: r.get(1)?,
                status: r.get(2)?,
                energy_required: r.get(3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
            }),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    match quest_opt {
        None => Ok(None),
        Some(quest) => {
            let steps = get_steps_for_quest(&conn, quest.id)?;
            Ok(Some(QuestWithSteps { quest, steps }))
        }
    }
}

fn get_steps_for_quest(conn: &Connection, quest_id: i64) -> Result<Vec<QuestStep>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, quest_id, body, done, step_order
             FROM quest_steps WHERE quest_id = ?1 ORDER BY step_order ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![quest_id], |r| Ok(QuestStep {
            id: r.get(0)?,
            quest_id: r.get(1)?,
            body: r.get(2)?,
            done: r.get::<_, i64>(3)? != 0,
            step_order: r.get(4)?,
        }))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
fn update_quest_status(
    state: State<DbState>,
    id: i64,
    status: String,
) -> Result<(), String> {
    let conn = state.inner().0.lock().unwrap();
    conn.execute(
        "UPDATE quests SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![status, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_quest(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.inner().0.lock().unwrap();
    conn.execute("DELETE FROM quests WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// --- Step commands ---

#[tauri::command]
fn create_step(
    state: State<DbState>,
    quest_id: i64,
    body: String,
    step_order: i64,
) -> Result<QuestStep, String> {
    let conn = state.inner().0.lock().unwrap();
    conn.execute(
        "INSERT INTO quest_steps (quest_id, body, step_order) VALUES (?1, ?2, ?3)",
        params![quest_id, body, step_order],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    conn.query_row(
        "SELECT id, quest_id, body, done, step_order FROM quest_steps WHERE id = ?1",
        params![id],
        |r| Ok(QuestStep {
            id: r.get(0)?,
            quest_id: r.get(1)?,
            body: r.get(2)?,
            done: r.get::<_, i64>(3)? != 0,
            step_order: r.get(4)?,
        }),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_step_done(state: State<DbState>, id: i64, done: bool) -> Result<(), String> {
    let conn = state.inner().0.lock().unwrap();
    conn.execute(
        "UPDATE quest_steps SET done = ?1 WHERE id = ?2",
        params![done as i64, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let conn = open_db(&app.handle()).expect("failed to open database");
            app.manage(DbState(Mutex::new(conn)));
            app.manage(VoiceState(Mutex::new(None)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_quest,
            get_quests,
            get_active_quest,
            update_quest_status,
            delete_quest,
            create_step,
            update_step_done,
            start_recognition,
            stop_recognition,
            model_downloaded,
            model_download::download_asr_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
