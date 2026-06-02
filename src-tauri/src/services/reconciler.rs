use chrono::{DateTime, Utc};
use diesel::prelude::*;
use tauri::AppHandle;
use tauri::Manager;

use crate::models::{CreateFocusHistoryInput, Quest, Reminder, Space};
use crate::schema::{focus_history, quests, reminders, spaces};
use crate::services::db::AppDbConnection;
use crate::services::device_connection::DeviceConnectionState;
use crate::services::notification;
use crate::services::quest::{emit_focus_enter_count_sync_event, QuestRepository};
use crate::services::reminder as reminder_svc;

/// Run on every app launch and after system resume.
///
/// 1. Snoozed reminders: re-arm in-process timers; fire past-due ones immediately.
/// 2. Past-due reminders: fire immediately + insert focus_history if not already recorded.
/// 3. Retroactive bridge: active quests with a due date but no Reminder row get one created
///    and scheduled (handles imports, multi-device bootstrap, pre-bridge data).
pub fn run(app: &AppHandle, db: &AppDbConnection) {
    notification::rearm_snoozed_reminders(app);
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
            .values((&input, focus_history::created_at.eq(&fire_time_str)))
            .execute(&mut *conn)
        {
            eprintln!(
                "[reconciler] failed to insert focus_history for {}: {e}",
                quest.id
            );
            continue;
        }

        let origin_device_id = app
            .try_state::<DeviceConnectionState>()
            .map(|device_connection| device_connection.identity.device_id.clone());
        if let Err(e) =
            record_reconciled_focus_enter(&mut conn, quest.clone(), origin_device_id.as_deref())
        {
            eprintln!(
                "[reconciler] failed to record focus_enter_count for {}: {e}",
                quest.id
            );
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

fn record_reconciled_focus_enter(
    conn: &mut SqliteConnection,
    quest: Quest,
    origin_device_id: Option<&str>,
) -> Result<(), String> {
    let (recorded, did_increment) =
        QuestRepository::new(conn).record_manual_focus_enter_transition(quest)?;
    if did_increment {
        if let Some(origin_device_id) = origin_device_id {
            emit_focus_enter_count_sync_event(conn, origin_device_id, &recorded)?;
        }
    }
    Ok(())
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

fn active_quests_without_reminder(conn: &mut SqliteConnection) -> Result<Vec<Quest>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::sync_outbox;
    use crate::services::db::open_db_at_path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("fini-{label}-{unique}.db"))
    }

    #[test]
    fn reconciled_reminder_focus_records_count_and_syncs() {
        let db_path = temp_db_path("reconciled-reminder-focus-count");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Past due reminder"),
                quests::status.eq("active"),
                quests::due.eq("2000-01-01"),
                quests::created_at.eq("2026-03-01T09:00:00Z"),
                quests::updated_at.eq("2026-03-01T09:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert active reminder quest");

        let quest: Quest = quests::table
            .filter(quests::title.eq("Past due reminder"))
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load active reminder quest");

        record_reconciled_focus_enter(&mut conn, quest.clone(), Some("device-test"))
            .expect("record reconciled focus enter");

        let focus_enter_count: i64 = quests::table
            .find(&quest.id)
            .select(quests::focus_enter_count)
            .first(&mut conn)
            .expect("load focus enter count");
        assert_eq!(focus_enter_count, 1);

        let event_count: i64 = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest_focus_enter_count"))
            .filter(sync_outbox::entity_id.eq(&quest.id))
            .count()
            .get_result(&mut conn)
            .expect("count focus sync events");
        assert_eq!(event_count, 1);

        record_reconciled_focus_enter(&mut conn, quest, Some("device-test"))
            .expect("record duplicate reconciled focus enter");
        let duplicate_event_count: i64 = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest_focus_enter_count"))
            .count()
            .get_result(&mut conn)
            .expect("count duplicate focus sync events");
        assert_eq!(duplicate_event_count, 1);

        let _ = std::fs::remove_file(db_path);
    }
}
