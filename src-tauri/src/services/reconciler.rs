use chrono::{DateTime, Utc};
use diesel::prelude::*;
use tauri::AppHandle;

use crate::models::{CreateFocusHistoryInput, Quest, Reminder, Space};
use crate::schema::{focus_history, quests, reminders, spaces};
use crate::services::db::DbState;
use crate::services::notification;
use crate::services::reminder as reminder_svc;

/// Run on every app launch.
///
/// 1. Past-due reminders: fire immediately + insert focus_history if not already recorded.
/// 2. Retroactive bridge: active quests with a due date but no Reminder row get one created
///    and scheduled (handles imports, multi-device bootstrap, pre-bridge data).
pub fn run(app: &AppHandle, db: &DbState) {
    let mut conn = db.0.lock().unwrap();
    let now = Utc::now();

    // ── Past-due: fire + record focus_history ────────────────────────────────
    let past_due: Vec<(Reminder, Quest, Space)> = match past_due_reminders(&mut conn, &now) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[reconciler] failed to query past-due reminders: {e}");
            return;
        }
    };

    for (reminder, quest, space) in &past_due {
        let already_recorded = focus_history_exists(&mut conn, &quest.id, &reminder);
        if already_recorded {
            continue;
        }

        let fire_time = reminder
            .due_at_utc
            .as_deref()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or(now);

        let fire_time_str = fire_time.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let input = CreateFocusHistoryInput {
            quest_id: quest.id.clone(),
            space_id: quest.space_id.clone(),
            trigger: "reminder".to_string(),
        };
        if let Err(e) = diesel::insert_into(focus_history::table)
            .values((
                &input,
                focus_history::created_at.eq(&fire_time_str),
            ))
            .execute(&mut *conn)
        {
            eprintln!("[reconciler] failed to insert focus_history for {}: {e}", quest.id);
            continue;
        }

        // Always fire immediately — no grace window
        notification::fire_immediate(app, reminder, quest, space);
    }

    // ── Retroactive bridge: active quests with due date but no Reminder row ──
    let unscheduled: Vec<Quest> = match active_quests_without_reminder(&mut conn) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[reconciler] failed to query unscheduled quests: {e}");
            return;
        }
    };

    for quest in &unscheduled {
        if let Err(e) = reminder_svc::upsert_reminder_for_quest(&mut conn, app, quest) {
            eprintln!("[reconciler] upsert_reminder failed for {}: {e}", quest.id);
        }
    }
}

fn past_due_reminders(
    conn: &mut SqliteConnection,
    now: &DateTime<Utc>,
) -> Result<Vec<(Reminder, Quest, Space)>, String> {
    let now_str = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let rows: Vec<(Reminder, Quest)> = reminders::table
        .inner_join(quests::table.on(reminders::quest_id.eq(quests::id)))
        .filter(reminders::due_at_utc.le(&now_str))
        .filter(quests::status.eq("active"))
        .select((Reminder::as_select(), Quest::as_select()))
        .load(conn)
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for (reminder, quest) in rows {
        let space: Space = spaces::table
            .find(&quest.space_id)
            .select(Space::as_select())
            .first(conn)
            .map_err(|e| e.to_string())?;
        result.push((reminder, quest, space));
    }
    Ok(result)
}

fn focus_history_exists(conn: &mut SqliteConnection, quest_id: &str, reminder: &Reminder) -> bool {
    let Some(due_str) = reminder.due_at_utc.as_deref() else {
        return false;
    };
    use diesel::dsl::count;
    focus_history::table
        .filter(focus_history::quest_id.eq(quest_id))
        .filter(focus_history::trigger.eq("reminder"))
        .filter(focus_history::created_at.eq(due_str))
        .select(count(focus_history::id))
        .first::<i64>(conn)
        .map(|c| c > 0)
        .unwrap_or(false)
}

fn active_quests_without_reminder(
    conn: &mut SqliteConnection,
) -> Result<Vec<Quest>, String> {
    let ids_with_reminders: Vec<String> = reminders::table
        .select(reminders::quest_id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    quests::table
        .filter(quests::status.eq("active"))
        .filter(quests::due.is_not_null())
        .filter(quests::id.ne_all(ids_with_reminders))
        .select(Quest::as_select())
        .load(conn)
        .map_err(|e| e.to_string())
}
