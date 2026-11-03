//! The quest commands: the UI's calls that read and change quests. A write
//! saves the quest and its sync outbox rows in one transaction, then
//! publishes `QuestChanged` or `QuestDeleted`; the reminders and sync react
//! to the event (docs/db.md, rules 4-7).

use std::sync::Arc;

use chrono::Utc;
use diesel::sqlite::SqliteConnection;
use tauri::{Manager, State};

use crate::models::{
    CreateQuestInput, EditScope, Event, Quest, QuestFieldPatch, QuestUpdatePatch, UpdateQuestCommandInput,
    UpdateQuestInputBuilder, OCCURRENCE_COMPLETED,
};
use crate::repositories::quest_repository::{append_focus_history, QuestChange};
use crate::repositories::{OutboxRepository, QuestRepository, ReminderRepository};
use crate::services::db::{DbPool, WriteTransaction};
use crate::services::event_bus_service::EventBus;
use crate::services::space_sync_service::SpaceSyncService;

type Bus<'a> = State<'a, Arc<EventBus>>;
type SpaceSync<'a> = State<'a, Arc<SpaceSyncService>>;

// ── Tauri commands ───────────────────────────────────────────────────────────

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_quests(state: State<DbPool>) -> Result<Vec<Quest>, String> {
    let mut conn = state.get()?;
    QuestRepository::new(&mut conn).all()
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_quest(pool: State<DbPool>, space_sync: SpaceSync, bus: Bus, input: CreateQuestInput) -> Result<Quest, String> {
    let device_id = &space_sync.identity().device_id;
    let quest = pool.get()?.write_transaction(|conn| {
        let quest = QuestRepository::new(conn).create(input)?;
        OutboxRepository::new(conn).record_quest(device_id, &quest.space_id, &quest.id)?;
        Ok(quest)
    })?;
    bus.publish(Event::QuestChanged { quest: quest.clone() });
    Ok(quest)
}

/// Complete a quest from a notification action. Logs errors rather than
/// returning them: the callers are notification action handlers. For a
/// repeating quest it closes the occurrence the reminder was for.
#[cfg(any(feature = "ui-plane", test))]
pub fn complete_quest_for_notification(app: &tauri::AppHandle, quest_id: &str) {
    let (Some(db), Some(space_sync), Some(bus)) = (
        app.try_state::<DbPool>(),
        app.try_state::<Arc<SpaceSyncService>>(),
        app.try_state::<Arc<EventBus>>(),
    ) else {
        eprintln!("[notification] complete: the app is not started yet");
        return;
    };

    let mut conn = db.connection();
    let device_id = space_sync.identity().device_id.clone();

    let quest: Quest = match QuestRepository::new(&mut conn).get(quest_id) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("[notification] complete: quest not found {quest_id}: {e}");
            return;
        }
    };
    let period_key = QuestRepository::new(&mut conn)
        .reminder_occurrence(quest_id)
        .ok()
        .flatten()
        .and_then(|day| day.due);

    let patch = QuestUpdatePatch {
        input: UpdateQuestInputBuilder::default().status(OCCURRENCE_COMPLETED).build(),
        description: QuestFieldPatch::Unchanged,
        due: QuestFieldPatch::Unchanged,
        due_time: QuestFieldPatch::Unchanged,
        repeat_rule: QuestFieldPatch::Unchanged,
    };
    let written = conn.write_transaction(|conn| {
        let change =
            QuestRepository::new(conn).update(quest_id, patch, EditScope::All, period_key.as_deref(), Some(&device_id))?;
        OutboxRepository::new(conn).record_change(&device_id, &quest.space_id, &change)?;
        Ok(change.quest)
    });
    match written {
        Ok(quest) => bus.publish(Event::QuestChanged { quest }),
        Err(e) => eprintln!("[notification] complete: update failed for {quest_id}: {e}"),
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_active_focus(state: State<DbPool>) -> Result<Option<Quest>, String> {
    let mut conn = state.get()?;
    QuestRepository::new(&mut conn).focus(Utc::now())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn set_focus(state: State<DbPool>, id: String) -> Result<Quest, String> {
    let mut conn = state.get()?;
    QuestRepository::new(&mut conn).set_focus(&id, "manual")
}

/// `input.scope` and `input.period_key` apply to a repeating quest only
/// (ADR-0012 D8).
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_quest(
    pool: State<DbPool>,
    space_sync: SpaceSync,
    bus: Bus,
    id: String,
    input: UpdateQuestCommandInput,
) -> Result<Quest, String> {
    let device_id = &space_sync.identity().device_id;
    let scope = EditScope::parse(input.scope.as_deref())?;
    let period_key = input.period_key.clone();
    write_change(&pool, &bus, device_id, &id, |conn| {
        let change =
            QuestRepository::new(conn).update(&id, input.into_patch(), scope, period_key.as_deref(), Some(device_id))?;
        if change.restore_should_focus {
            append_focus_history(conn, &change.quest.id, &change.quest.space_id, "restore")?;
        }
        Ok(change)
    })
}

/// "Complete through today" (ADR-0012 D7).
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn complete_quest_through_today(pool: State<DbPool>, space_sync: SpaceSync, bus: Bus, id: String) -> Result<Quest, String> {
    let device_id = &space_sync.identity().device_id;
    write_change(&pool, &bus, device_id, &id, |conn| {
        QuestRepository::new(conn).complete_through_today(&id, Some(device_id))
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_quest(pool: State<DbPool>, space_sync: SpaceSync, bus: Bus, id: String) -> Result<(), String> {
    let reminders = pool.get()?.write_transaction(|conn| {
        let quest = QuestRepository::new(conn).get(&id)?;
        let reminders = ReminderRepository::new(conn).list_for_quest(&id)?;
        OutboxRepository::new(conn).emit_sync_event(
            &space_sync.identity().device_id,
            "quest",
            &id,
            &quest.space_id,
            "delete",
            None,
        )?;
        QuestRepository::new(conn).delete(&id)?;
        Ok(reminders)
    })?;
    bus.publish(Event::QuestDeleted { quest_id: id, reminders });
    Ok(())
}

/// Writes one change of quest `id` and its outbox rows in one transaction,
/// then publishes `QuestChanged`. `write` makes the change on the
/// transaction's connection.
#[cfg(any(feature = "ui-plane", test))]
fn write_change(
    pool: &DbPool,
    bus: &EventBus,
    device_id: &str,
    id: &str,
    write: impl FnOnce(&mut SqliteConnection) -> Result<QuestChange, String>,
) -> Result<Quest, String> {
    let quest = pool.get()?.write_transaction(|conn| {
        let previous_space_id = QuestRepository::new(conn).get(id)?.space_id;
        let change = write(conn)?;
        OutboxRepository::new(conn).record_change(device_id, &previous_space_id, &change)?;
        Ok(change.quest)
    })?;
    bus.publish(Event::QuestChanged { quest: quest.clone() });
    Ok(quest)
}

// ── Checklist commands (issue #128) ─────────────────────────────────────────

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn add_checklist_item(
    pool: State<DbPool>,
    space_sync: SpaceSync,
    bus: Bus,
    quest_id: String,
    text: String,
    scope: Option<String>,
) -> Result<Quest, String> {
    let device_id = &space_sync.identity().device_id;
    let scope = EditScope::parse(scope.as_deref())?;
    write_change(&pool, &bus, device_id, &quest_id, |conn| {
        QuestRepository::new(conn).add_checklist_item(&quest_id, &text, scope, Some(device_id))
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn toggle_checklist_item(
    pool: State<DbPool>,
    space_sync: SpaceSync,
    bus: Bus,
    quest_id: String,
    item_id: String,
    checked: bool,
    period_key: Option<String>,
) -> Result<Quest, String> {
    write_change(&pool, &bus, &space_sync.identity().device_id, &quest_id, |conn| {
        QuestRepository::new(conn).toggle_checklist_item(&quest_id, &item_id, checked, period_key.as_deref())
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn edit_checklist_item(
    pool: State<DbPool>,
    space_sync: SpaceSync,
    bus: Bus,
    quest_id: String,
    item_id: String,
    text: String,
    scope: Option<String>,
) -> Result<Quest, String> {
    let device_id = &space_sync.identity().device_id;
    let scope = EditScope::parse(scope.as_deref())?;
    write_change(&pool, &bus, device_id, &quest_id, |conn| {
        QuestRepository::new(conn).edit_checklist_item_text(&quest_id, &item_id, &text, scope, Some(device_id))
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn remove_checklist_item(
    pool: State<DbPool>,
    space_sync: SpaceSync,
    bus: Bus,
    quest_id: String,
    item_id: String,
    scope: Option<String>,
) -> Result<Quest, String> {
    let device_id = &space_sync.identity().device_id;
    let scope = EditScope::parse(scope.as_deref())?;
    write_change(&pool, &bus, device_id, &quest_id, |conn| {
        QuestRepository::new(conn).remove_checklist_item(&quest_id, &item_id, scope, Some(device_id))
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn reorder_checklist(
    pool: State<DbPool>,
    space_sync: SpaceSync,
    bus: Bus,
    quest_id: String,
    ordered_item_ids: Vec<String>,
    scope: Option<String>,
) -> Result<Quest, String> {
    let scope = EditScope::parse(scope.as_deref())?;
    write_change(&pool, &bus, &space_sync.identity().device_id, &quest_id, |conn| {
        QuestRepository::new(conn).reorder_checklist(&quest_id, &ordered_item_ids, scope)
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_checklist_activity(
    state: State<DbPool>,
    quest_id: String,
) -> Result<Vec<crate::models::ChecklistActivity>, String> {
    let mut conn = state.get()?;
    QuestRepository::new(&mut conn).get_checklist_activity(&quest_id)
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Test helper: one update through the service; (quest as the UI shows it,
/// whether a restore should set Focus, ()).
#[cfg(test)]
pub fn update_quest_in_db(
    conn: &mut SqliteConnection,
    id: &str,
    input: crate::models::UpdateQuestInput,
) -> Result<(Quest, bool, ()), String> {
    let change = QuestRepository::new(conn).update(
        id,
        crate::models::QuestUpdatePatch::unchanged(input),
        EditScope::All,
        None,
        None,
    )?;
    Ok((change.quest, change.restore_should_focus, ()))
}

#[cfg(test)]
pub(crate) fn load_quests_for_list(
    conn: &mut SqliteConnection,
) -> Result<Vec<Quest>, diesel::result::Error> {
    use diesel::prelude::*;
    let loaded: Vec<Quest> = crate::schema::quests::table.select(Quest::as_select()).load(conn)?;
    Ok(crate::services::cli::sort_quests_for_list(loaded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{period_key_of, validate_metadata, RepeatingQuest, UpdateQuestInput};
    use crate::repositories::quest_repository::log_checklist_activity;
    use crate::schema::{focus_history, quests};
    use crate::models::QuestOccurrence;
    use crate::services::checklist;
    use diesel::prelude::*;

    #[test]
    fn quest_service_metadata_validation_rejects_values_outside_persistence_domain() {
        assert!(validate_metadata(1, 3).is_ok());
        assert_eq!(
            validate_metadata(0, 2).unwrap_err(),
            "invalid stored energy 0; expected 1, 2, or 3"
        );
        assert_eq!(
            validate_metadata(2, 4).unwrap_err(),
            "invalid stored priority 4; expected 1, 2, or 3"
        );
    }
    use crate::models::{clamp_order_rank, QuestFieldPatch};
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

    fn is_uuid_like(value: &str) -> bool {
        value.len() == 36
            && value.as_bytes()[8] == b'-'
            && value.as_bytes()[13] == b'-'
            && value.as_bytes()[18] == b'-'
            && value.as_bytes()[23] == b'-'
    }

    fn insert_active_quest(
        conn: &mut SqliteConnection,
        title: &str,
        priority: i64,
        created_at: &str,
        due: Option<&str>,
        due_time: Option<&str>,
    ) -> String {
        use crate::schema::quests;
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

    fn status_patch(status: &str) -> UpdateQuestInput {
        UpdateQuestInput {
            space_id: None,
            title: None,
            description: None,
            status: Some(status.to_string()),
            energy: None,
            priority: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            is_checklist: None,
        }
    }

    fn due_patch(due: &str) -> UpdateQuestInput {
        UpdateQuestInput {
            space_id: None,
            title: None,
            description: None,
            status: None,
            energy: None,
            priority: None,
            due: Some(due.to_string()),
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            is_checklist: None,
        }
    }

    fn space_patch(space_id: &str) -> UpdateQuestInput {
        UpdateQuestInput {
            space_id: Some(space_id.to_string()),
            title: None,
            description: None,
            status: None,
            energy: None,
            priority: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            is_checklist: None,
        }
    }

    #[test]
    fn nullable_patch_clears_each_quest_field_and_omission_preserves_values() {
        let db_path = temp_db_path("quest-nullable-patch");
        let mut conn = open_db_at_path(&db_path);
        let id = insert_active_quest(
            &mut conn,
            "nullable patch",
            1,
            "2026-03-28T10:00:00Z",
            Some("2026-04-01"),
            Some("10:30"),
        );
        diesel::update(quests::table.find(&id))
            .set((
                quests::description.eq(Some("keep or clear")),
                quests::repeat_rule.eq(Some(r#"{"preset":"weekly"}"#)),
            ))
            .execute(&mut conn)
            .expect("seed nullable fields");

        let patch = QuestUpdatePatch {
            input: status_patch("active"),
            description: QuestFieldPatch::Clear,
            due: QuestFieldPatch::Clear,
            due_time: QuestFieldPatch::Clear,
            repeat_rule: QuestFieldPatch::Clear,
        };
        let cleared = QuestRepository::new(&mut conn)
            .update(&id, patch, EditScope::All, None, None)
            .expect("clear nullable fields");
        assert_eq!(cleared.quest.description, None);
        assert_eq!(cleared.quest.due, None);
        assert_eq!(cleared.quest.due_time, None);
        assert_eq!(cleared.quest.repeat_rule, None);

        diesel::update(quests::table.find(&id))
            .set((
                quests::description.eq(Some("preserved")),
                quests::due.eq(Some("2026-05-01")),
                quests::due_time.eq(Some("08:00")),
                quests::repeat_rule.eq(Some("weekly")),
            ))
            .execute(&mut conn)
            .expect("reseed nullable fields");
        let preserved = QuestRepository::new(&mut conn)
            .update(&id, QuestUpdatePatch::unchanged(status_patch("active")), EditScope::All, None, None)
            .expect("omit nullable fields");
        assert_eq!(preserved.quest.description.as_deref(), Some("preserved"));
        assert_eq!(preserved.quest.due.as_deref(), Some("2026-05-01"));
        assert_eq!(preserved.quest.due_time.as_deref(), Some("08:00"));
        assert_eq!(preserved.quest.repeat_rule.as_deref(), Some("weekly"));

        let empty = QuestRepository::new(&mut conn)
            .update(
                &id,
                QuestUpdatePatch {
                    input: status_patch("active"),
                    description: QuestFieldPatch::Set(String::new()),
                    due: QuestFieldPatch::Unchanged,
                    due_time: QuestFieldPatch::Unchanged,
                    repeat_rule: QuestFieldPatch::Unchanged,
                },
                EditScope::All,
                None,
                None,
            )
            .expect("set an empty nullable field");
        assert_eq!(empty.quest.description.as_deref(), Some(""));
        assert_eq!(empty.quest.due.as_deref(), Some("2026-05-01"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn set_focus_records_manual_history_and_touches() {
        let db_path = temp_db_path("set-focus-service");
        let mut conn = open_db_at_path(&db_path);
        let id = insert_active_quest(
            &mut conn,
            "focus through service",
            1,
            "2020-01-01T00:00:00Z",
            None,
            None,
        );

        let focused = QuestRepository::new(&mut conn)
            .set_focus(&id, "manual")
            .expect("set active quest as focus");

        assert_eq!(focused.id, id);
        // "Focus N" counting is switched off for now (see the note at the top).
        assert_ne!(focused.updated_at, "2020-01-01T00:00:00Z");
        let triggers: Vec<String> = focus_history::table
            .filter(focus_history::quest_id.eq(&id))
            .select(focus_history::trigger)
            .load(&mut conn)
            .expect("load focus history");
        assert_eq!(triggers, vec!["manual"]);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn order_rank_is_clamped_to_signed_100() {
        assert_eq!(clamp_order_rank(150.0), 100.0);
        assert_eq!(clamp_order_rank(-150.0), -100.0);
        assert_eq!(clamp_order_rank(42.5), 42.5);
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
    fn manual_set_focus_overrides_fallback_selection() {
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
            3,
            "2026-03-02T10:00:00Z",
            None,
            None,
        );

        let before = QuestRepository::new(&mut conn).focus(Utc::now())
            .expect("resolve before set-main")
            .expect("must return active quest");
        assert_eq!(
            before.id, high_priority_id,
            "without overrides, fallback should pick higher priority"
        );

        // Insert a focus_history row for the low-priority quest (simulates manual Focus set)
        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '2026-03-03T12:00:00Z')",
            low_priority_id
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let after = QuestRepository::new(&mut conn).focus(Utc::now())
            .expect("resolve after set-main")
            .expect("must return active quest");
        assert_eq!(
            after.id, low_priority_id,
            "focus_history entry must override fallback ordering"
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

        // Manual focus set at 09:00
        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '2026-03-05T09:00:00Z')",
            manual_id
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let before_preempt = QuestRepository::new(&mut conn).focus(Utc::now())
            .expect("resolve before reminder")
            .expect("must return active quest");
        assert_eq!(
            before_preempt.id, manual_id,
            "manual focus_history entry should be active before reminder"
        );

        // Reminder fires at 09:30 — newer than manual, so it preempts
        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'reminder', '2026-03-05T09:30:00Z')",
            reminder_id
        ))
        .execute(&mut conn)
        .expect("insert reminder focus_history row");

        let preempted = QuestRepository::new(&mut conn).focus(Utc::now())
            .expect("resolve during reminder")
            .expect("must return active quest");
        assert_eq!(
            preempted.id, reminder_id,
            "newest focus_history entry (reminder) should preempt manual Focus"
        );

        // Resolve the reminder quest — it becomes inactive
        diesel::update(quests::table.find(&reminder_id))
            .set(quests::status.eq("completed"))
            .execute(&mut conn)
            .expect("resolve reminder quest by completion");

        // Resolver skips completed quest, finds manual entry for manual_id
        let unwound = QuestRepository::new(&mut conn).focus(Utc::now())
            .expect("resolve after reminder completion")
            .expect("must return active quest");
        assert_eq!(
            unwound.id, manual_id,
            "after reminder quest resolves, Focus should unwind to previous valid active entry"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn due_reminder_competes_with_manual_focus_by_timestamp() {
        let db_path = temp_db_path("due-reminder-competes-with-manual-focus-by-timestamp");
        let mut conn = open_db_at_path(&db_path);

        let manual_id = insert_active_quest(
            &mut conn,
            "manual-focus-before-reminder",
            2,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let reminder_id = insert_active_quest(
            &mut conn,
            "future-reminder-focus",
            2,
            "2026-03-01T11:00:00Z",
            Some("2026-03-05"),
            Some("09:31"),
        );

        let reminder_quest: Quest = quests::table
            .find(&reminder_id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load reminder quest");
        let fire_at = reminder_quest.due_at().expect("the moment the quest comes");
        let manual_at = fire_at - chrono::Duration::minutes(1);
        let manual_at_str = manual_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '{}')",
            manual_id, manual_at_str
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let before_due = QuestRepository::new(&mut conn).focus(fire_at - chrono::Duration::seconds(1))
            .expect("resolve before due")
            .expect("must return active quest");
        assert_eq!(
            before_due.id, manual_id,
            "future reminder must not preempt manual Focus before its fire time"
        );

        let after_due = QuestRepository::new(&mut conn).focus(fire_at)
            .expect("resolve after due")
            .expect("must return active quest");
        assert_eq!(
            after_due.id, reminder_id,
            "due reminder timestamp must preempt older manual Focus"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn newer_manual_focus_beats_already_due_reminder() {
        let db_path = temp_db_path("newer-manual-focus-beats-already-due-reminder");
        let mut conn = open_db_at_path(&db_path);

        let manual_id = insert_active_quest(
            &mut conn,
            "manual-focus-after-reminder",
            2,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let reminder_id = insert_active_quest(
            &mut conn,
            "already-due-reminder-focus",
            2,
            "2026-03-01T11:00:00Z",
            Some("2026-03-05"),
            Some("09:31"),
        );

        let reminder_quest: Quest = quests::table
            .find(&reminder_id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load reminder quest");
        let fire_at = reminder_quest.due_at().expect("the moment the quest comes");
        let manual_at = fire_at + chrono::Duration::minutes(1);
        let manual_at_str = manual_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '{}')",
            manual_id, manual_at_str
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let winner = QuestRepository::new(&mut conn).focus(manual_at + chrono::Duration::seconds(1))
            .expect("resolve after manual")
            .expect("must return active quest");
        assert_eq!(
            winner.id, manual_id,
            "newer manual Focus timestamp must beat an older due reminder timestamp"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_order_is_overdue_then_priority_then_oldest() {
        let db_path = temp_db_path("fallback-order-overdue-priority-oldest");
        let mut conn = open_db_at_path(&db_path);

        let _overdue_low = insert_active_quest(
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
            3,
            "2026-03-02T08:00:00Z",
            Some("2000-01-02"),
            None,
        );
        let _future_urgent = insert_active_quest(
            &mut conn,
            "future-urgent",
            3,
            "2026-03-03T08:00:00Z",
            Some("2999-01-01"),
            None,
        );

        let first = QuestRepository::new(&mut conn).focus(Utc::now())
            .expect("resolve with mixed overdue state")
            .expect("must return active quest");
        assert_eq!(
            first.id, overdue_urgent,
            "among overdue quests, higher priority should win"
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
            3,
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

        let winner = QuestRepository::new(&mut conn).focus(Utc::now())
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

        let winner = QuestRepository::new(&mut conn).focus(Utc::now())
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
    fn update_quest_command_input_null_actually_clears_due_due_time_and_repeat_rule() {
        let db_path = temp_db_path("update-command-input-clears-fields");
        let mut conn = open_db_at_path(&db_path);

        let id = insert_active_quest(
            &mut conn,
            "Clearable reminder",
            2,
            "2026-04-06T10:00:00Z",
            Some("2026-04-10"),
            Some("09:00"),
        );
        diesel::update(quests::table.find(&id))
            .set(quests::repeat_rule.eq(Some(r#"{"preset":"weekly"}"#)))
            .execute(&mut conn)
            .expect("seed repeat_rule");

        let command_input: crate::models::UpdateQuestCommandInput = serde_json::from_str(
            r#"{"due":null,"due_time":null,"repeat_rule":null}"#,
        )
        .expect("clear payload must deserialize");
        QuestRepository::new(&mut conn)
            .update(&id, command_input.into_patch(), EditScope::All, None, None)
            .expect("clearing update must succeed");

        let cleared: Quest = quests::table
            .find(&id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload cleared quest");
        assert_eq!(cleared.due, None, "explicit null must clear due");
        assert_eq!(cleared.due_time, None, "explicit null must clear due_time");
        assert_eq!(
            cleared.repeat_rule, None,
            "explicit null must clear repeat_rule"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn update_quest_command_input_omitted_fields_leave_due_and_repeat_rule_untouched() {
        let db_path = temp_db_path("update-command-input-preserves-omitted-fields");
        let mut conn = open_db_at_path(&db_path);

        let id = insert_active_quest(
            &mut conn,
            "Untouched reminder",
            2,
            "2026-04-06T10:00:00Z",
            Some("2026-04-10"),
            Some("09:00"),
        );
        diesel::update(quests::table.find(&id))
            .set(quests::repeat_rule.eq(Some(r#"{"preset":"weekly"}"#)))
            .execute(&mut conn)
            .expect("seed repeat_rule");

        let command_input: crate::models::UpdateQuestCommandInput =
            serde_json::from_str(r#"{"priority":"high"}"#)
                .expect("partial payload must deserialize");
        QuestRepository::new(&mut conn)
            .update(&id, command_input.into_patch(), EditScope::All, None, None)
            .expect("partial update must succeed");

        let updated: Quest = quests::table
            .find(&id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload updated quest");
        assert_eq!(
            updated.priority,
            crate::models::PRIORITY_HIGH,
            "priority must be applied"
        );
        assert_eq!(
            updated.due.as_deref(),
            Some("2026-04-10"),
            "omitted due must be left unchanged"
        );
        assert_eq!(
            updated.due_time.as_deref(),
            Some("09:00"),
            "omitted due_time must be left unchanged"
        );
        assert_eq!(
            updated.repeat_rule.as_deref(),
            Some(r#"{"preset":"weekly"}"#),
            "omitted repeat_rule must be left unchanged"
        );

        let _ = std::fs::remove_file(db_path);
    }




    #[test]
    fn restoring_future_due_quest_does_not_set_focus() {
        let db_path = temp_db_path("restore-future-due-does-not-focus-main");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Future restore"),
                quests::status.eq("completed"),
                quests::due.eq("2999-01-01"),
                quests::completed_at.eq("2026-03-01T09:05:00Z"),
                quests::created_at.eq("2026-03-01T09:00:00Z"),
                quests::updated_at.eq("2026-03-01T09:05:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert completed future-due quest");

        let id: String = quests::table
            .filter(quests::title.eq("Future restore"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load inserted quest id");

        let (restored, restore_should_focus, _) =
            update_quest_in_db(&mut conn, &id, status_patch("active"))
                .expect("restore future-due quest");

        assert_eq!(restored.status, "active");
        assert!(
            !restore_should_focus,
            "future-due restore must not write a focus_history entry"
        );

        let focus_count: i64 = focus_history::table
            .filter(focus_history::quest_id.eq(&id))
            .count()
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(
            focus_count, 0,
            "future-due restore must leave focus_history empty"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn restoring_past_due_quest_appends_focus_history() {
        let db_path = temp_db_path("restore-past-due-focuses-main");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Past restore"),
                quests::status.eq("completed"),
                quests::due.eq("2000-01-01"),
                quests::created_at.eq("2026-03-01T09:00:00Z"),
                quests::updated_at.eq("2026-03-01T09:05:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert completed past-due quest");

        let id: String = quests::table
            .filter(quests::title.eq("Past restore"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load inserted quest id");

        let (restored, restore_should_focus, _) =
            update_quest_in_db(&mut conn, &id, status_patch("active"))
                .expect("restore past-due quest");

        assert_eq!(restored.status, "active");
        assert!(
            restore_should_focus,
            "past-due restore must signal that a focus_history entry should be written"
        );

        let _ = std::fs::remove_file(db_path);
    }



    #[test]
    fn moving_quest_between_spaces_emits_delete_then_upsert_events() {
        let db_path = temp_db_path("moving-quest-between-spaces-emits-delete-then-upsert");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("space-move-events"),
                quests::status.eq("active"),
                quests::created_at.eq("2026-04-01T00:00:00Z"),
                quests::updated_at.eq("2026-04-01T00:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert quest");

        let id: String = quests::table
            .filter(quests::title.eq("space-move-events"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load quest id");

        let (updated, _, _) =
            update_quest_in_db(&mut conn, &id, space_patch("2")).expect("move quest to space 2");
        OutboxRepository::new(&mut conn).record_quest("device-local", "1", &updated.id)
            .expect("emit sync events for space move");

        let rows: Vec<(String, String, String, Option<String>)> = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest"))
            .filter(sync_outbox::entity_id.eq(&id))
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::op_type,
                sync_outbox::space_id,
                sync_outbox::updated_at,
                sync_outbox::payload,
            ))
            .load(&mut conn)
            .expect("load emitted outbox rows");

        assert_eq!(rows.len(), 2, "space move must emit delete + upsert");
        assert_eq!(rows[0].0, "delete");
        assert_eq!(rows[0].1, "1");
        assert!(rows[0].3.is_none(), "delete event payload must be empty");
        assert_eq!(rows[1].0, "upsert");
        assert_eq!(rows[1].1, "2");
        assert!(rows[1].3.is_some(), "upsert event payload must be present");
        assert!(
            rows[1].2 > rows[0].2,
            "upsert event must be newer than delete event"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn checklist_activity_is_emitted_with_checklist_sync() {
        let db_path = temp_db_path("checklist-activity-sync-outbox");
        let mut conn = open_db_at_path(&db_path);

        let quest = QuestRepository::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Pack".to_string(),
                description: Some("- [ ] headphones <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create checklist quest");

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        OutboxRepository::new(&mut conn).record_quest("device-local", &quest.space_id, &quest.id)
            .expect("emit quest sync with checklist activity");

        let rows: Vec<(String, String, String, Option<String>)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::entity_type,
                sync_outbox::entity_id,
                sync_outbox::op_type,
                sync_outbox::payload,
            ))
            .load(&mut conn)
            .expect("load sync outbox rows");

        assert_eq!(rows.len(), 2, "quest and activity events should be emitted");
        assert_eq!(rows[0].0, "quest");
        assert_eq!(rows[0].1, quest.id);
        assert_eq!(rows[0].2, "upsert");
        assert_eq!(rows[1].0, "checklist_activity");
        assert_eq!(rows[1].2, "upsert");
        let payload = rows[1].3.as_ref().expect("activity payload");
        let activity: crate::models::ChecklistActivity = serde_json::from_str(payload).unwrap();
        assert_eq!(activity.quest_id, quest.id);
        assert_eq!(activity.kind, "added");
        assert_eq!(activity.detail, "Added \"headphones\"");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn checklist_sync_does_not_reemit_existing_activity_on_toggle() {
        let db_path = temp_db_path("checklist-activity-sync-no-reemit");
        let mut conn = open_db_at_path(&db_path);

        let quest = QuestRepository::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Pack".to_string(),
                description: Some("- [ ] headphones <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create checklist quest");

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        OutboxRepository::new(&mut conn).record_quest("device-local", &quest.space_id, &quest.id)
            .expect("emit initial quest sync with checklist activity");

        let toggled = QuestRepository::new(&mut conn)
            .toggle_checklist_item(&quest.id, "a1", true, None)
            .expect("toggle checklist item")
            .quest;
        OutboxRepository::new(&mut conn).record_quest("device-local", &toggled.space_id, &toggled.id)
            .expect("emit toggle quest sync without duplicate activity");

        let rows: Vec<(String, String)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((sync_outbox::entity_type, sync_outbox::op_type))
            .load(&mut conn)
            .expect("load sync outbox rows");

        assert_eq!(
            rows.len(),
            3,
            "initial quest/activity plus toggle quest only"
        );
        assert_eq!(rows[0], ("quest".to_string(), "upsert".to_string()));
        assert_eq!(
            rows[1],
            ("checklist_activity".to_string(), "upsert".to_string())
        );
        assert_eq!(rows[2], ("quest".to_string(), "upsert".to_string()));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn moving_checklist_to_new_space_reemits_activity_for_destination_space() {
        let db_path = temp_db_path("checklist-activity-sync-space-move-reemit");
        let mut conn = open_db_at_path(&db_path);

        let quest = QuestRepository::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Pack".to_string(),
                description: Some("- [ ] headphones <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create checklist quest");

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        OutboxRepository::new(&mut conn).record_quest("device-local", &quest.space_id, &quest.id)
            .expect("emit initial quest sync with checklist activity");

        let (moved, _, _) = update_quest_in_db(&mut conn, &quest.id, space_patch("2"))
            .expect("move checklist to space 2");
        OutboxRepository::new(&mut conn).record_quest("device-local", &quest.space_id, &moved.id)
            .expect("emit move sync with checklist activity in destination space");

        let rows: Vec<(String, String, String)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::entity_type,
                sync_outbox::op_type,
                sync_outbox::space_id,
            ))
            .load(&mut conn)
            .expect("load sync outbox rows");

        assert_eq!(
            rows,
            vec![
                ("quest".to_string(), "upsert".to_string(), "1".to_string()),
                (
                    "checklist_activity".to_string(),
                    "upsert".to_string(),
                    "1".to_string()
                ),
                ("quest".to_string(), "delete".to_string(), "1".to_string()),
                ("quest".to_string(), "upsert".to_string(), "2".to_string()),
                (
                    "checklist_activity".to_string(),
                    "upsert".to_string(),
                    "2".to_string()
                ),
            ],
            "space move must re-emit checklist activity under the destination space"
        );

        let _ = std::fs::remove_file(db_path);
    }





    // ── Checklist (issue #128) ──────────────────────────────────────────────
    //
    // Checklist content lives in the existing `description` field, parsed/rendered as task-list
    // text when `is_checklist` is set — there is no dedicated checklist content column.

    fn create_active_quest_with_checklist(conn: &mut SqliteConnection, checklist: &str) -> Quest {
        let result = QuestRepository::new(conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(checklist.to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create quest with checklist");
        result
    }

    fn create_active_quest_with_prose(conn: &mut SqliteConnection, description: &str) -> Quest {
        let result = QuestRepository::new(conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Meeting notes".to_string(),
                description: Some(description.to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: false,
            })
            .expect("create quest with prose");
        result
    }

    #[test]
    fn checklist_item_crud_logs_activity_but_toggles_do_not() {
        let db_path = temp_db_path("checklist-crud");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_checklist(&mut conn, "");
        let mut service = QuestRepository::new(&mut conn);

        let quest = service
            .add_checklist_item(&quest.id, "headphones", EditScope::All, Some("dev-a"))
            .expect("add item")
            .quest;
        let item_id = checklist::parse(quest.description.as_deref().unwrap())[0]
            .id
            .clone();

        let quest = service
            .toggle_checklist_item(&quest.id, &item_id, true, None)
            .expect("toggle item")
            .quest;
        assert!(checklist::parse(quest.description.as_deref().unwrap())[0].checked);

        let quest = service
            .edit_checklist_item_text(&quest.id, &item_id, "over-ear headphones", EditScope::All, Some("dev-a"))
            .expect("edit item text")
            .quest;
        assert_eq!(
            checklist::parse(quest.description.as_deref().unwrap())[0].text,
            "over-ear headphones"
        );

        let quest = service
            .remove_checklist_item(&quest.id, &item_id, EditScope::All, Some("dev-a"))
            .expect("remove item")
            .quest;
        assert!(checklist::parse(quest.description.as_deref().unwrap()).is_empty());

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        let kinds: Vec<&str> = activity.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["added", "edited", "removed"],
            "toggling checked state must not write an activity row"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn generic_description_patch_is_rejected_on_a_checklist_quest() {
        let db_path = temp_db_path("checklist-reject-prose-patch");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: true,
        }]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);

        // `quest update --description "notes"` must not be able to silently overwrite the
        // checklist text with prose, which would lose every item's id and checked state.
        let mut service = QuestRepository::new(&mut conn);
        let result = service.update(
            &quest.id,
            QuestUpdatePatch::unchanged(UpdateQuestInput {
                description: Some("notes".to_string()),
                ..status_patch("active")
            }),
            EditScope::All,
            None,
            None,
        );
        assert!(
            result.is_err(),
            "a plain description patch must be rejected for a checklist quest"
        );

        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(
            stored.description.as_deref(),
            Some(template.as_str()),
            "rejected patch must not touch the stored checklist"
        );

        // The nullable-patch path (CLI's `quest update --description`) must reject the same way.
        let patch_result = service.update(
            &quest.id,
            QuestUpdatePatch {
                input: status_patch("active"),
                description: QuestFieldPatch::Set("notes".to_string()),
                due: QuestFieldPatch::Unchanged,
                due_time: QuestFieldPatch::Unchanged,
                repeat_rule: QuestFieldPatch::Unchanged,
            },
            EditScope::All,
            None,
            None,
        );
        assert!(
            patch_result.is_err(),
            "the nullable-patch path must reject a description Set on a checklist quest too"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn toggling_missing_checklist_item_is_rejected_without_writing() {
        let db_path = temp_db_path("checklist-toggle-missing-item");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[
            checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: true,
            },
        ]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);
        let original_description = quest.description.clone();
        let original_updated_at = quest.updated_at.clone();

        let result = QuestRepository::new(&mut conn).toggle_checklist_item(&quest.id, "missing", true, None);
        assert!(matches!(
            result,
            Err(ref error) if error == "checklist item not found"
        ));

        let mut service = QuestRepository::new(&mut conn);
        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(stored.description, original_description);
        assert_eq!(stored.updated_at, original_updated_at);

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(activity.is_empty(), "rejected toggle must not log activity");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn editing_missing_checklist_item_text_is_rejected_without_writing() {
        let db_path = temp_db_path("checklist-edit-missing-item");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);
        let original_description = quest.description.clone();

        let result = QuestRepository::new(&mut conn).edit_checklist_item_text(
            &quest.id,
            "missing",
            "new text",
            EditScope::All,
            Some("dev-a"),
        );
        assert!(matches!(
            result,
            Err(ref error) if error == "checklist item not found"
        ));

        let mut service = QuestRepository::new(&mut conn);
        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(
            stored.description, original_description,
            "rejected edit must not rewrite the checklist"
        );

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.is_empty(),
            "rejected edit must not log a false 'Renamed item' activity"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn removing_missing_checklist_item_is_rejected_without_writing() {
        let db_path = temp_db_path("checklist-remove-missing-item");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);
        let original_description = quest.description.clone();

        let result =
            QuestRepository::new(&mut conn).remove_checklist_item(&quest.id, "missing", EditScope::All, Some("dev-a"));
        assert!(matches!(
            result,
            Err(ref error) if error == "checklist item not found"
        ));

        let mut service = QuestRepository::new(&mut conn);
        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(
            stored.description, original_description,
            "rejected remove must not rewrite the checklist"
        );

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.is_empty(),
            "rejected remove must not log a false 'Removed an item' activity"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn toggling_non_active_checklist_item_is_rejected_without_writing() {
        for status in ["completed", "abandoned"] {
            let db_path = temp_db_path(&format!("checklist-toggle-non-active-{status}"));
            let mut conn = open_db_at_path(&db_path);

            let template = checklist::serialize(&[checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            }]);
            let quest = create_active_quest_with_checklist(&mut conn, &template);
            let mut service = QuestRepository::new(&mut conn);
            service
                .update(&quest.id, QuestUpdatePatch::unchanged(status_patch(status)), EditScope::All, None, None)
                .expect("move quest out of active status");
            let before = service
                .get(&quest.id)
                .expect("load non-active checklist quest");
            let before_activity = service
                .get_checklist_activity(&quest.id)
                .expect("load activity before rejected toggle");

            let result = service.toggle_checklist_item(&quest.id, "a1", true, None);
            assert!(matches!(
                result,
                Err(ref error) if error == "checklist toggles are only available for active quests"
            ));

            let after = service
                .get(&quest.id)
                .expect("reload non-active checklist quest");
            assert_eq!(after.description, before.description);
            assert_eq!(after.updated_at, before.updated_at);

            let after_activity = service
                .get_checklist_activity(&quest.id)
                .expect("load activity after rejected toggle");
            assert_eq!(
                after_activity.len(),
                before_activity.len(),
                "rejected non-active toggle must not add audit rows"
            );

            let _ = std::fs::remove_file(db_path);
        }
    }

    #[test]
    fn checklist_mutations_reject_prose_quests_without_conversion() {
        let db_path = temp_db_path("checklist-reject-prose");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_prose(&mut conn, "meeting notes");
        let mut service = QuestRepository::new(&mut conn);

        for result in [
            service.add_checklist_item(&quest.id, "headphones", EditScope::All, Some("dev-a")).map(|_| ()),
            service.toggle_checklist_item(&quest.id, "a1", true, None).map(|_| ()),
            service.edit_checklist_item_text(&quest.id, "a1", "headphones", EditScope::All, Some("dev-a")).map(|_| ()),
            service.remove_checklist_item(&quest.id, "a1", EditScope::All, Some("dev-a")).map(|_| ()),
            service.reorder_checklist(&quest.id, &["a1".to_string()], EditScope::All).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(ref error) if error == "quest is not a checklist"
            ));
        }

        let stored = service.get(&quest.id).expect("load prose quest");
        assert!(
            !stored.is_checklist,
            "mutation must not convert prose quests"
        );
        assert_eq!(stored.description.as_deref(), Some("meeting notes"));

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.is_empty(),
            "rejected mutations must not log activity"
        );

        let _ = std::fs::remove_file(db_path);
    }


    #[test]
    fn completing_quest_with_checklist_logs_completed_snapshot() {
        let db_path = temp_db_path("checklist-completion-snapshot");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[
            checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: true,
            },
            checklist::ChecklistItem {
                id: "a2".into(),
                text: "lunch".into(),
                checked: false,
            },
        ]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);

        let mut service = QuestRepository::new(&mut conn);
        service
            .complete(&quest.id, Some("dev-a"))
            .expect("complete quest");

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        let snapshot = activity
            .iter()
            .find(|a| a.kind == "completed_snapshot")
            .expect("completed_snapshot activity row must exist");
        assert_eq!(snapshot.detail, "Checklist at completion: 1/2 checked");
        assert_eq!(snapshot.origin_device_id.as_deref(), Some("dev-a"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn editing_checklist_after_completion_logs_post_completion_edit() {
        let db_path = temp_db_path("checklist-post-completion-edit");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_checklist(&mut conn, "");
        let mut service = QuestRepository::new(&mut conn);
        service
            .complete(&quest.id, None)
            .expect("complete quest");

        service
            .add_checklist_item(&quest.id, "late addition", EditScope::All, Some("dev-a"))
            .expect("add item after completion");

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.iter().any(|a| a.kind == "post_completion_edit"),
            "post-completion checklist edits must be recorded as historical activity, per #128"
        );
        assert!(
            !activity.iter().any(|a| a.kind == "added"),
            "post-completion edits use the distinct kind, not the normal 'added' kind"
        );

        let _ = std::fs::remove_file(db_path);
    }

    // ── Repeating quests (ADR-0012) ─────────────────────────────────────────

    fn day(offset_days: i64) -> String {
        period_key_of(chrono::Local::now().date_naive() + chrono::Duration::days(offset_days))
    }

    fn create_repeating(conn: &mut SqliteConnection, due: &str, rule: &str, checklist: Option<&str>) -> Quest {
        QuestRepository::new(conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Gym".to_string(),
                description: checklist.map(str::to_string),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some(due.to_string()),
                due_time: Some("20:00".to_string()),
                repeat_rule: Some(rule.to_string()),
                order_rank: None,
                is_checklist: checklist.is_some(),
            })
            .expect("create repeating quest")
    }

    const DAILY: &str = r#"{"preset":"daily"}"#;

    fn marks(conn: &mut SqliteConnection, quest_id: &str) -> Vec<(String, Option<String>)> {
        use crate::schema::quest_occurrences;
        quest_occurrences::table
            .filter(quest_occurrences::quest_id.eq(quest_id))
            .order(quest_occurrences::period_key.asc())
            .select((quest_occurrences::period_key, quest_occurrences::status))
            .load(conn)
            .expect("load marks")
    }

    #[test]
    fn a_repeating_quest_stores_no_day_rows_until_a_day_is_closed() {
        let db_path = temp_db_path("repeating-no-rows");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-2), DAILY, None);

        let listed = QuestRepository::new(&mut conn).all().expect("list");
        assert_eq!(listed.len(), 1, "one row for the quest, its oldest open day");
        assert_eq!(listed[0].due.as_deref(), Some(day(-2).as_str()));
        assert!(marks(&mut conn, &quest.id).is_empty(), "nothing stored while nobody acts");

        let change = QuestRepository::new(&mut conn).complete(&quest.id, None).expect("complete");
        assert_eq!(marks(&mut conn, &quest.id), vec![(day(-2), Some("completed".to_string()))]);
        assert_eq!(change.quest.due.as_deref(), Some(day(-1).as_str()), "the next open day shows");
        assert_eq!(change.quest.status, "active");
        let quest_rows: i64 = quests::table.count().get_result(&mut conn).unwrap();
        assert_eq!(quest_rows, 1, "complete writes no next occurrence row");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn complete_through_today_closes_each_open_day_through_today() {
        let db_path = temp_db_path("repeating-complete-through-today");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-3), DAILY, None);

        let change = QuestRepository::new(&mut conn).complete_through_today(&quest.id, None).expect("complete through today");
        let expected: Vec<_> = (-3..=0).map(|offset| (day(offset), Some("completed".to_string()))).collect();
        assert_eq!(marks(&mut conn, &quest.id), expected, "one completed mark per day, today's included");
        assert_eq!(change.occurrences.len(), 4);
        assert_eq!(change.quest.due.as_deref(), Some(day(1).as_str()));

        let again = QuestRepository::new(&mut conn).complete_through_today(&quest.id, None);
        assert!(again.is_err(), "nothing is left to close through today");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_repeating_day_comes_to_focus_at_its_time() {
        let db_path = temp_db_path("repeating-focus-future-day");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-2), DAILY, None);
        QuestRepository::new(&mut conn).set_focus(&quest.id, "manual").expect("set focus");
        assert_eq!(
            QuestRepository::new(&mut conn).focus(Utc::now()).expect("resolve").map(|q| q.id),
            Some(quest.id.clone()),
            "a missed day keeps the quest on Focus"
        );

        QuestRepository::new(&mut conn).complete_through_today(&quest.id, None).expect("complete through today");
        assert_eq!(
            QuestRepository::new(&mut conn).focus(Utc::now()).expect("resolve").map(|q| q.id),
            None,
            "the next day is tomorrow at 20:00; it has not come yet"
        );

        let comes_at = crate::services::due_time::compute_fire_utc(&day(1), Some("20:00")).expect("moment");
        assert_eq!(
            QuestRepository::new(&mut conn).focus(comes_at - chrono::Duration::minutes(1)).expect("resolve").map(|q| q.id),
            None,
            "one minute before tomorrow's 20:00"
        );
        assert_eq!(
            QuestRepository::new(&mut conn).focus(comes_at).expect("resolve").map(|q| q.id),
            Some(quest.id.clone()),
            "tomorrow at 20:00 the day comes to Focus"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn reopening_a_closed_day_removes_its_empty_mark() {
        let db_path = temp_db_path("repeating-reopen");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-1), DAILY, None);
        QuestRepository::new(&mut conn).complete(&quest.id, None).expect("complete");

        let reopened = QuestRepository::new(&mut conn)
            .update(
                &quest.id,
                QuestUpdatePatch::unchanged(status_patch("active")),
                EditScope::All,
                Some(&day(-1)),
                None,
            )
            .expect("reopen the day");
        assert_eq!(reopened.removed_occurrences, vec![day(-1)]);
        assert!(marks(&mut conn, &quest.id).is_empty());
        assert_eq!(reopened.quest.due.as_deref(), Some(day(-1).as_str()), "the reopened day is the oldest open again");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_day_edit_overrides_one_day_and_an_all_days_edit_changes_every_day() {
        let db_path = temp_db_path("repeating-edit-scope");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-1), DAILY, None);
        let rename = |title: &str| {
            QuestUpdatePatch::unchanged(UpdateQuestInput { title: Some(title.to_string()), ..status_patch("active") })
        };
        let mut no_status = rename("Gym (short)");
        no_status.input.status = None;

        QuestRepository::new(&mut conn)
            .update(&quest.id, no_status, EditScope::Day, None, None)
            .expect("this day");
        let mut service = QuestRepository::new(&mut conn);
        assert_eq!(service.get(&quest.id).unwrap().title, "Gym", "the quest keeps its title");
        assert_eq!(service.resolved(&quest.id).unwrap().title, "Gym (short)", "the day shows its own title");

        let mut all = rename("Run");
        all.input.status = None;
        service.update(&quest.id, all, EditScope::All, None, None).expect("all days");
        assert_eq!(service.get(&quest.id).unwrap().title, "Run");
        assert_eq!(service.resolved(&quest.id).unwrap().title, "Gym (short)", "the day keeps its override");
        service.complete(&quest.id, None).expect("complete the overridden day");
        assert_eq!(service.resolved(&quest.id).unwrap().title, "Run", "the next day takes the quest's title");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_tick_writes_a_day_copy_and_a_template_edit_reaches_it() {
        let db_path = temp_db_path("repeating-checklist-day-copy");
        let mut conn = open_db_at_path(&db_path);
        let template = checklist::serialize(&[
            checklist::ChecklistItem { id: "a1".into(), text: "headphones".into(), checked: false },
            checklist::ChecklistItem { id: "a2".into(), text: "charger".into(), checked: false },
        ]);
        let quest = create_repeating(&mut conn, &day(0), DAILY, Some(&template));

        let ticked = QuestRepository::new(&mut conn).toggle_checklist_item(&quest.id, "a1", true, None).expect("tick");
        assert!(checklist::parse(ticked.quest.description.as_deref().unwrap())[0].checked);
        let mut service = QuestRepository::new(&mut conn);
        assert_eq!(service.get(&quest.id).unwrap().description.as_deref(), Some(template.as_str()), "the template has no ticks");

        let added = service.add_checklist_item(&quest.id, "lunch", EditScope::All, None).expect("add to all days").quest;
        let items = checklist::parse(added.description.as_deref().unwrap());
        assert_eq!(items.len(), 3, "the open day copy took the template edit");
        assert!(items.iter().find(|it| it.id == "a1").unwrap().checked, "and kept its tick");
        assert!(!items.iter().find(|it| it.text == "lunch").unwrap().checked);

        service.complete(&quest.id, None).expect("complete the day");
        let next = service.resolved(&quest.id).unwrap();
        assert!(checklist::parse(next.description.as_deref().unwrap()).iter().all(|it| !it.checked), "the next day starts unticked");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn stopping_the_repeat_ends_the_rule_at_the_last_closed_day() {
        let db_path = temp_db_path("repeating-stop");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-2), DAILY, None);
        QuestRepository::new(&mut conn).complete(&quest.id, None).expect("complete one day");

        let mut stop = QuestUpdatePatch::unchanged(status_patch("active"));
        stop.input.status = None;
        stop.repeat_rule = QuestFieldPatch::Clear;
        QuestRepository::new(&mut conn).update(&quest.id, stop, EditScope::All, None, None).expect("stop the repeat");

        let mut service = QuestRepository::new(&mut conn);
        let stored = service.get(&quest.id).unwrap();
        assert_eq!(stored.status, "completed", "the quest ends");
        assert!(stored.repeat_rule.as_deref().unwrap().contains(&day(-2)), "the rule ends at the last closed day");
        assert!(service.active_quests().unwrap().is_empty(), "no open day is left");
        assert_eq!(marks(&mut conn, &quest.id).len(), 1, "the closed day stays in History");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_rule_that_ends_after_n_has_no_open_day_after_n_completions() {
        let db_path = temp_db_path("repeating-end-after-n");
        let mut conn = open_db_at_path(&db_path);
        let rule = r#"{"preset":"daily","end":"after_n","end_after":2}"#;
        let quest = create_repeating(&mut conn, &day(-5), rule, None);

        QuestRepository::new(&mut conn).complete_through_today(&quest.id, None).expect("complete through today");
        assert_eq!(marks(&mut conn, &quest.id).len(), 2, "only the two days the rule allows close");
        assert!(QuestRepository::new(&mut conn).active_quests().unwrap().is_empty());

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn custom_rules_compute_their_dates() {
        let quest_with = |due: &str, rule: &str| Quest {
            due: Some(due.to_string()),
            repeat_rule: Some(rule.to_string()),
            ..test_quest_row()
        };
        let none: Vec<QuestOccurrence> = Vec::new();

        // Every 2 weeks on Monday and Wednesday, from Monday 2026-03-02.
        let quest = quest_with("2026-03-02", r#"{"preset":"custom","interval":2,"days_of_week":["Mo","We"]}"#);
        let repeating = RepeatingQuest::new(&quest, &none).unwrap();
        let dates: Vec<String> = repeating.dates().take(4).map(period_key_of).collect();
        assert_eq!(dates, ["2026-03-02", "2026-03-04", "2026-03-16", "2026-03-18"]);

        // Every 3 days, with no weekdays: the unit step.
        let quest = quest_with("2026-03-01", r#"{"preset":"custom","interval":3,"unit":"day"}"#);
        let repeating = RepeatingQuest::new(&quest, &none).unwrap();
        let dates: Vec<String> = repeating.dates().take(3).map(period_key_of).collect();
        assert_eq!(dates, ["2026-03-01", "2026-03-04", "2026-03-07"]);
    }

    fn test_quest_row() -> Quest {
        Quest {
            id: "q".to_string(),
            space_id: "1".to_string(),
            title: "Gym".to_string(),
            description: None,
            status: "active".to_string(),
            energy: crate::models::ENERGY_MEDIUM,
            priority: crate::models::PRIORITY_MEDIUM,
            due: None,
            due_time: None,
            repeat_rule: None,
            completed_at: None,
            order_rank: 0.0,
            focus_enter_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            is_checklist: false,
            checklist_base: None,
        }
    }
}
