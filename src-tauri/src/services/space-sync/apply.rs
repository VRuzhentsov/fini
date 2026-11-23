use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use super::UnresolvedCustomSpace;
use crate::models::{ChecklistActivity, FocusHistoryEntry, Quest, QuestOccurrence, Space};
use crate::repositories::QuestRepository;
use crate::repositories::{OutboxRepository, SpaceMappingRepository};
use crate::schema::{checklist_activity, focus_history, spaces, sync_outbox, tombstones};
use crate::services::channel_service::{CustomSpaceDescriptor, SyncEventEnvelope};
use crate::services::db::utc_now;

pub(super) fn normalized_space_ids(mapped_space_ids: Vec<String>) -> Vec<String> {
    mapped_space_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn is_builtin_space_id(space_id: &str) -> bool {
    matches!(space_id, "1" | "2" | "3")
}

pub(super) fn placeholder_space_name(space_id: &str) -> String {
    let short = if space_id.len() > 8 {
        &space_id[..8]
    } else {
        space_id
    };
    format!("Shared {short}")
}

fn ensure_spaces_exist(conn: &mut SqliteConnection, desired_ids: &[String]) -> Result<(), String> {
    if desired_ids.is_empty() {
        return Ok(());
    }

    let existing_space_ids: Vec<String> = spaces::table
        .filter(spaces::id.eq_any(desired_ids))
        .select(spaces::id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let existing: BTreeSet<String> = existing_space_ids.into_iter().collect();
    let missing: Vec<String> = desired_ids
        .iter()
        .filter(|id| !existing.contains((*id).as_str()))
        .cloned()
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    let first_order = spaces::table
        .select(diesel::dsl::max(spaces::item_order))
        .first::<Option<i64>>(conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
        + 1;

    for (order, space_id) in (first_order..).zip(missing) {
        diesel::insert_or_ignore_into(spaces::table)
            .values((
                spaces::id.eq(&space_id),
                spaces::name.eq(placeholder_space_name(&space_id)),
                spaces::item_order.eq(order),
            ))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn missing_space_ids(
    conn: &mut SqliteConnection,
    desired_ids: &[String],
) -> Result<Vec<String>, String> {
    if desired_ids.is_empty() {
        return Ok(Vec::new());
    }

    let existing_space_ids: Vec<String> = spaces::table
        .filter(spaces::id.eq_any(desired_ids))
        .select(spaces::id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let existing: BTreeSet<String> = existing_space_ids.into_iter().collect();
    Ok(desired_ids
        .iter()
        .filter(|id| !existing.contains((*id).as_str()))
        .cloned()
        .collect())
}

pub(super) fn load_custom_space_descriptors(
    conn: &mut SqliteConnection,
    mapped_space_ids: &[String],
) -> Result<Vec<CustomSpaceDescriptor>, String> {
    let custom_ids: Vec<String> = mapped_space_ids
        .iter()
        .filter(|id| !is_builtin_space_id(id))
        .cloned()
        .collect();

    if custom_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows: Vec<Space> = spaces::table
        .filter(spaces::id.eq_any(&custom_ids))
        .select(Space::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    let mut by_id = std::collections::HashMap::new();
    for row in rows {
        by_id.insert(row.id.clone(), row.name.clone());
    }

    Ok(custom_ids
        .into_iter()
        .filter_map(|space_id| {
            by_id.get(&space_id).map(|name| CustomSpaceDescriptor {
                space_id,
                name: name.clone(),
            })
        })
        .collect())
}

/// Sets the spaces this pair syncs to `mapped_space_ids`: new ones start,
/// missing ones end. Returns the spaces it syncs after that.
pub(super) fn apply_mappings(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if !crate::repositories::DeviceRepository::new(conn).is_paired(peer_device_id) {
        return Err("paired device not found".to_string());
    }
    let desired_ids = normalized_space_ids(mapped_space_ids);
    let missing = missing_space_ids(conn, &desired_ids)?;
    if !missing.is_empty() {
        return Err(format!("unknown space ids: {}", missing.join(", ")));
    }
    let existing: BTreeSet<String> = SpaceMappingRepository::new(conn)
        .active(peer_device_id)?
        .into_iter()
        .collect();
    let desired: BTreeSet<String> = desired_ids.iter().cloned().collect();
    let to_add: Vec<String> = desired.difference(&existing).cloned().collect();
    let to_remove: Vec<String> = existing.difference(&desired).cloned().collect();

    // Synced again before the peer heard that it had ended: the end no
    // longer holds, and arriving late it would undo this.
    for space_id in &to_add {
        OutboxRepository::new(conn).drop_space_sync_end(peer_device_id, space_id)?;
    }
    let mut mappings = SpaceMappingRepository::new(conn);
    mappings.end(peer_device_id, &to_remove, &utc_now())?;
    mappings.start(peer_device_id, &to_add)?;
    mappings.active(peer_device_id)
}

fn upsert_space(conn: &mut SqliteConnection, space: &Space) -> Result<(), String> {
    diesel::insert_into(spaces::table)
        .values((
            spaces::id.eq(&space.id),
            spaces::name.eq(&space.name),
            spaces::item_order.eq(space.item_order),
            spaces::created_at.eq(&space.created_at),
        ))
        .on_conflict(spaces::id)
        .do_update()
        .set((
            spaces::name.eq(&space.name),
            spaces::item_order.eq(space.item_order),
            spaces::created_at.eq(&space.created_at),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn upsert_quest(conn: &mut SqliteConnection, quest: &Quest) -> Result<(), String> {
    ensure_spaces_exist(conn, std::slice::from_ref(&quest.space_id))?;
    QuestRepository::new(conn).sync_upsert_quest(quest)
}

/// An occurrence mark of a repeating quest (ADR-0012 D11). A tick applies
/// its quest first; a mark that arrives before its quest waits for a later
/// tick.
fn upsert_quest_occurrence(
    conn: &mut SqliteConnection,
    occurrence: &QuestOccurrence,
) -> Result<(), String> {
    QuestRepository::new(conn).sync_upsert_occurrence(occurrence)
}

fn upsert_focus_history(
    conn: &mut SqliteConnection,
    focus: &FocusHistoryEntry,
) -> Result<(), String> {
    ensure_spaces_exist(conn, std::slice::from_ref(&focus.space_id))?;

    diesel::insert_into(focus_history::table)
        .values((
            focus_history::id.eq(&focus.id),
            focus_history::quest_id.eq(&focus.quest_id),
            focus_history::space_id.eq(&focus.space_id),
            focus_history::trigger.eq(&focus.trigger),
            focus_history::created_at.eq(&focus.created_at),
        ))
        .on_conflict(focus_history::id)
        .do_update()
        .set((
            focus_history::quest_id.eq(&focus.quest_id),
            focus_history::space_id.eq(&focus.space_id),
            focus_history::trigger.eq(&focus.trigger),
            focus_history::created_at.eq(&focus.created_at),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_entity(conn: &mut SqliteConnection, event: &SyncEventEnvelope) -> Result<(), String> {
    diesel::insert_or_ignore_into(tombstones::table)
        .values((
            tombstones::entity_type.eq(&event.entity_type),
            tombstones::entity_id.eq(&event.entity_id),
            tombstones::space_id.eq(&event.space_id),
            tombstones::deleted_at.eq(utc_now()),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    match event.entity_type.as_str() {
        "space" => {
            diesel::delete(spaces::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        "quest" => {
            let mut quests = QuestRepository::new(conn);
            if quests.find_external_quest(&event.entity_id)?.is_some() {
                quests.delete(&event.entity_id)?;
            }
        }
        "quest_occurrence" => {
            if let Some((quest_id, period_key)) = event.entity_id.split_once(':') {
                QuestRepository::new(conn).delete_occurrence(quest_id, period_key)?;
            }
        }
        "focus_history" => {
            diesel::delete(focus_history::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        "checklist_activity" => {
            diesel::delete(checklist_activity::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn cleanup_old_tombstones(conn: &mut SqliteConnection) -> Result<usize, String> {
    let cutoff = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::days(30))
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let deleted = diesel::delete(tombstones::table.filter(tombstones::deleted_at.lt(&cutoff)))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(deleted)
}

/// Our own device id, read straight from `settings` (mirrors
/// `pairing::runtime::DEVICE_ID_KEY`) so the checklist merge push-back can stamp an
/// accurate `origin_device_id` without needing app-state (`apply_sync_event` only has `conn`).
fn local_device_id(conn: &mut SqliteConnection) -> Option<String> {
    crate::services::settings::load_setting(conn, "device.id")
        .ok()
        .flatten()
}

fn emit_checklist_convergence_event(
    conn: &mut SqliteConnection,
    remote_origin_device_id: &str,
    entity_id: &str,
) -> Result<(), String> {
    if let Some(own_device_id) = local_device_id(conn) {
        if own_device_id != remote_origin_device_id {
            let refreshed = QuestRepository::new(conn).get(entity_id)?;
            let payload = serde_json::to_string(&refreshed).map_err(|e| e.to_string())?;
            OutboxRepository::new(conn).emit_sync_event(
                &own_device_id,
                "quest",
                entity_id,
                &refreshed.space_id,
                "upsert",
                Some(payload),
            )?;
        }
    }
    Ok(())
}

/// Sends a merged day mark back out, so that the peer that sent the other
/// side of the merge gets the same checklist.
fn emit_occurrence_convergence_event(
    conn: &mut SqliteConnection,
    remote_origin_device_id: &str,
    space_id: &str,
    mark: &QuestOccurrence,
) -> Result<(), String> {
    match local_device_id(conn) {
        Some(own_device_id) if own_device_id != remote_origin_device_id => {
            OutboxRepository::new(conn).record_occurrence(&own_device_id, space_id, mark)
        }
        _ => Ok(()),
    }
}

/// Merges an incoming checklist quest's `description` (task-list text) against local state
/// and persists the result, regardless of whether the incoming event wins the whole-entity LWW
/// comparison for the quest's other fields. Issue #128 explicitly forbids whole-quest LWW for
/// checklist item state — this is the retained-base 3-way merge from the storage spike
/// (`services::checklist::merge_3way`). Only called when the local quest is a checklist quest
/// (`is_checklist`); plain prose quests never enter this path and follow normal whole-entity LWW
/// on `description` like any other field. Pushes the merged result back out to the origin peer
/// when it differs from what they sent, so both sides converge. Returns the merged description
/// for the caller to fold into `upsert_quest`.
fn merge_and_persist_quest_checklist(
    conn: &mut SqliteConnection,
    entity_id: &str,
    incoming_description: Option<&str>,
) -> Result<(Option<String>, bool), String> {
    QuestRepository::new(conn).sync_merge_quest_checklist(entity_id, incoming_description)
}

pub(super) fn apply_sync_event(
    conn: &mut SqliteConnection,
    event: &SyncEventEnvelope,
) -> Result<bool, String> {
    if OutboxRepository::new(conn).is_event_seen(&event.event_id)? {
        return Ok(false);
    }

    match event.op_type.as_str() {
        "delete" => {
            delete_entity(conn, event)?;
        }
        "upsert" => {
            let whole_entity_wins = match OutboxRepository::new(conn)
                .load_latest_event_for_entity(&event.entity_type, &event.entity_id)?
            {
                Some(local_event) => event.wins_over(&local_event),
                None => true,
            };

            let payload = event
                .payload
                .as_ref()
                .ok_or_else(|| "missing payload for upsert event".to_string())?;
            let payload = payload.as_str();

            // Checklist items get a per-item merge instead of following whole-quest LWW (#128),
            // so a locally-known checklist quest is handled before the generic win/lose gate
            // below. A quest that isn't (yet) known locally as a checklist quest — including a
            // brand-new quest arriving for the first time — falls through to the normal
            // whole-entity path; its `description` is treated like any other field.
            let local_is_checklist = if event.entity_type == "quest" {
                QuestRepository::new(conn).external_quest_is_checklist(&event.entity_id)?
            } else {
                false
            };
            // A day mark of a checklist quest gets the same per-item merge
            // (ADR-0012 D5), also before the win/lose gate.
            let checklist_occurrence = if event.entity_type == "quest_occurrence" {
                let occurrence: QuestOccurrence =
                    serde_json::from_str(payload).map_err(|e| e.to_string())?;
                QuestRepository::new(conn)
                    .external_quest_is_checklist(&occurrence.quest_id)?
                    .then_some(occurrence)
            } else {
                None
            };

            if let Some(occurrence) = checklist_occurrence {
                let converged = QuestRepository::new(conn)
                    .sync_merge_occurrence_checklist(&occurrence, whole_entity_wins)?;
                if let Some(mark) = converged {
                    emit_occurrence_convergence_event(
                        conn,
                        &event.origin_device_id,
                        &event.space_id,
                        &mark,
                    )?;
                }
                if !whole_entity_wins {
                    OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
                    return Ok(false);
                }
            } else if local_is_checklist {
                let mut quest: Quest = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                let (merged_description, should_emit_convergence) =
                    merge_and_persist_quest_checklist(
                        conn,
                        &event.entity_id,
                        quest.description.as_deref(),
                    )?;

                if whole_entity_wins {
                    quest.description = merged_description;
                    quest.is_checklist = true;
                    upsert_quest(conn, &quest)?;
                    if should_emit_convergence {
                        emit_checklist_convergence_event(
                            conn,
                            &event.origin_device_id,
                            &event.entity_id,
                        )?;
                    }
                } else {
                    if should_emit_convergence {
                        emit_checklist_convergence_event(
                            conn,
                            &event.origin_device_id,
                            &event.entity_id,
                        )?;
                    }
                    OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
                    return Ok(false);
                }
            } else if !whole_entity_wins {
                OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
                return Ok(false);
            } else {
                match event.entity_type.as_str() {
                    "space" => {
                        let space: Space =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_space(conn, &space)?;
                    }
                    "quest" => {
                        // Not a (locally-known) checklist quest — normal whole-entity upsert,
                        // description included, exactly as before #128.
                        let quest: Quest =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_quest(conn, &quest)?;
                    }
                    "quest_occurrence" => {
                        let occurrence: QuestOccurrence =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_quest_occurrence(conn, &occurrence)?;
                    }
                    "focus_history" => {
                        let focus: FocusHistoryEntry =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_focus_history(conn, &focus)?;
                    }
                    "checklist_activity" => {
                        let activity: ChecklistActivity =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        diesel::insert_or_ignore_into(checklist_activity::table)
                            .values((
                                checklist_activity::id.eq(&activity.id),
                                checklist_activity::quest_id.eq(&activity.quest_id),
                                checklist_activity::kind.eq(&activity.kind),
                                checklist_activity::detail.eq(&activity.detail),
                                checklist_activity::created_at.eq(&activity.created_at),
                                checklist_activity::origin_device_id.eq(&activity.origin_device_id),
                            ))
                            .execute(conn)
                            .map_err(|e| e.to_string())?;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
    SpaceMappingRepository::new(conn).record_synced(
        &event.origin_device_id,
        &event.space_id,
        &event.updated_at,
    )?;
    Ok(true)
}

fn next_space_item_order(conn: &mut SqliteConnection) -> Result<i64, String> {
    let value = spaces::table
        .select(diesel::dsl::max(spaces::item_order))
        .first::<Option<i64>>(conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
        + 1;
    Ok(value)
}

pub(super) fn create_space_with_id(
    conn: &mut SqliteConnection,
    space_id: &str,
    name: &str,
) -> Result<(), String> {
    let order = next_space_item_order(conn)?;
    diesel::insert_or_ignore_into(spaces::table)
        .values((
            spaces::id.eq(space_id),
            spaces::name.eq(name),
            spaces::item_order.eq(order),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn merge_local_space_into_remote_id(
    conn: &mut SqliteConnection,
    local_space_id: &str,
    remote_space_id: &str,
    remote_space_name: Option<&str>,
) -> Result<(), String> {
    if local_space_id == remote_space_id {
        return Ok(());
    }

    if is_builtin_space_id(local_space_id) {
        return Err("cannot merge built-in spaces into custom remote id".to_string());
    }

    let local_space: Space = spaces::table
        .find(local_space_id)
        .select(Space::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;

    let target_exists = spaces::table
        .find(remote_space_id)
        .count()
        .get_result::<i64>(conn)
        .map_err(|e| e.to_string())?
        > 0;

    if !target_exists {
        let name = remote_space_name
            .map(str::to_string)
            .unwrap_or_else(|| local_space.name.clone());
        create_space_with_id(conn, remote_space_id, &name)?;
    }

    QuestRepository::new(conn).move_quests_between_spaces(local_space_id, remote_space_id)?;

    diesel::update(focus_history::table.filter(focus_history::space_id.eq(local_space_id)))
        .set(focus_history::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    diesel::update(sync_outbox::table.filter(sync_outbox::space_id.eq(local_space_id)))
        .set(sync_outbox::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    SpaceMappingRepository::new(conn).move_space(local_space_id, remote_space_id)?;

    diesel::delete(spaces::table.find(local_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub(super) fn partition_remote_mapped_spaces(
    conn: &mut SqliteConnection,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<(Vec<String>, Vec<UnresolvedCustomSpace>), String> {
    let desired_ids = normalized_space_ids(mapped_space_ids);
    let missing = missing_space_ids(conn, &desired_ids)?;
    if missing.is_empty() {
        return Ok((desired_ids, Vec::new()));
    }

    let missing_set: BTreeSet<String> = missing.into_iter().collect();
    let mut custom_name_by_id = std::collections::HashMap::new();
    for item in custom_spaces {
        custom_name_by_id.insert(item.space_id, item.name);
    }

    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();

    for space_id in desired_ids {
        if !missing_set.contains(&space_id) {
            resolved.push(space_id);
            continue;
        }

        if is_builtin_space_id(&space_id) {
            ensure_spaces_exist(conn, std::slice::from_ref(&space_id))?;
            resolved.push(space_id);
            continue;
        }

        let name = custom_name_by_id
            .get(&space_id)
            .cloned()
            .unwrap_or_else(|| placeholder_space_name(&space_id));
        unresolved.push(UnresolvedCustomSpace { space_id, name });
    }

    Ok((resolved, unresolved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateSyncOutboxEntry;
    use crate::schema::{pair_space_mappings, paired_devices, quest_occurrences, quests};
    use crate::services::db::open_db_at_path;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fini-test-cmd-{label}-{}.db", Uuid::new_v4()))
    }

    fn test_quest(id: &str, title: &str, updated_at: &str) -> Quest {
        Quest {
            id: id.to_string(),
            space_id: "1".to_string(),
            title: title.to_string(),
            description: None,
            status: "active".to_string(),
            energy: crate::models::ENERGY_MEDIUM,
            priority: crate::models::PRIORITY_MEDIUM,
            due: None,
            due_time: None,
            repeat_rule: None,
            completed_at: None,
            order_rank: "a0".to_string(),
            focus_enter_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
            is_checklist: false,
            checklist_base: None,
        }
    }

    fn test_envelope(
        event_id: &str,
        origin_device_id: &str,
        entity_type: &str,
        entity_id: &str,
        op_type: &str,
        payload: Option<String>,
        updated_at: &str,
    ) -> SyncEventEnvelope {
        SyncEventEnvelope {
            event_id: event_id.to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            origin_device_id: origin_device_id.to_string(),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            space_id: "1".to_string(),
            op_type: op_type.to_string(),
            payload,
            updated_at: updated_at.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn quest_payload(quest: &Quest) -> String {
        serde_json::to_string(quest).unwrap()
    }

    fn insert_outbox_entry_for_space(
        conn: &mut SqliteConnection,
        origin_device_id: &str,
        entity_type: &str,
        entity_id: &str,
        space_id: &str,
        updated_at: &str,
        payload: Option<String>,
    ) -> String {
        let event_id = Uuid::new_v4().to_string();
        let entry = CreateSyncOutboxEntry {
            event_id: event_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            origin_device_id: origin_device_id.to_string(),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            space_id: space_id.to_string(),
            op_type: "upsert".to_string(),
            payload,
            updated_at: updated_at.to_string(),
        };
        diesel::insert_into(sync_outbox::table)
            .values(&entry)
            .execute(conn)
            .unwrap();
        event_id
    }

    fn insert_outbox_entry(
        conn: &mut SqliteConnection,
        origin_device_id: &str,
        entity_type: &str,
        entity_id: &str,
        updated_at: &str,
        payload: Option<String>,
    ) {
        let _ = insert_outbox_entry_for_space(
            conn,
            origin_device_id,
            entity_type,
            entity_id,
            "1",
            updated_at,
            payload,
        );
    }

    #[test]
    fn apply_sync_event_deduplicates_by_event_id() {
        let db_path = temp_db_path("dedup");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let quest = test_quest("q1", "Test", "2026-03-01T00:00:00Z");
        let event = test_envelope(
            "evt-1",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&quest)),
            "2026-03-01T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());
        assert!(!apply_sync_event(&mut conn, &event).unwrap());

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_preserves_checklist_activity_rows() {
        let db_path = temp_db_path("checklist-activity-sync");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let mut quest = test_quest("q-activity", "Packed", "2026-03-01T00:00:00Z");
        quest.is_checklist = true;
        insert_local_quest_row(&mut conn, &quest);

        let activity = ChecklistActivity {
            id: "activity-1".to_string(),
            quest_id: quest.id.clone(),
            kind: "completed_snapshot".to_string(),
            detail: "Checklist at completion: 1/2 checked".to_string(),
            created_at: "2026-03-01T01:00:00Z".to_string(),
            origin_device_id: Some("dev-remote".to_string()),
        };
        let event = test_envelope(
            "evt-activity",
            "dev-remote",
            "checklist_activity",
            &activity.id,
            "upsert",
            Some(serde_json::to_string(&activity).unwrap()),
            "2026-03-01T01:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let stored: ChecklistActivity = checklist_activity::table
            .find(&activity.id)
            .select(ChecklistActivity::as_select())
            .first(&mut conn)
            .expect("synced activity should be stored");
        assert_eq!(stored.quest_id, quest.id);
        assert_eq!(stored.kind, "completed_snapshot");
        assert_eq!(stored.detail, "Checklist at completion: 1/2 checked");
        assert_eq!(stored.origin_device_id.as_deref(), Some("dev-remote"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_respects_conflict_resolution() {
        let db_path = temp_db_path("conflict");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let local_quest = test_quest("q1", "Local", "2026-03-02T00:00:00Z");
        insert_outbox_entry(
            &mut conn,
            "dev-local",
            "quest",
            "q1",
            "2026-03-02T00:00:00Z",
            Some(quest_payload(&local_quest)),
        );

        // Incoming older event should lose
        let old_quest = test_quest("q1", "Remote Old", "2026-03-01T00:00:00Z");
        let old_event = test_envelope(
            "evt-old",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&old_quest)),
            "2026-03-01T00:00:00Z",
        );
        assert!(
            !apply_sync_event(&mut conn, &old_event).unwrap(),
            "older incoming event should be rejected"
        );

        // Incoming newer event should win
        let new_quest = test_quest("q1", "Remote New", "2026-03-03T00:00:00Z");
        let new_event = test_envelope(
            "evt-new",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&new_quest)),
            "2026-03-03T00:00:00Z",
        );
        assert!(
            apply_sync_event(&mut conn, &new_event).unwrap(),
            "newer incoming event should be applied"
        );

        let _ = std::fs::remove_file(db_path);
    }

    /// Inserts a real local `quests` row (not just an outbox entry) so the checklist merge path,
    /// which reads the row directly, has something to merge against.
    /// ADR-0012 D11: a peer's occurrence mark applies to its quest, keyed by
    /// (quest_id, period_key); its delete removes it.
    #[test]
    fn apply_sync_event_applies_and_deletes_an_occurrence_mark() {
        let db_path = temp_db_path("occurrence-mark-sync");
        let mut conn = open_db_at_path(&db_path);
        let mut quest = test_quest("quest-repeating", "Gym", "2026-03-01T00:00:00Z");
        quest.repeat_rule = Some(r#"{"preset":"daily"}"#.to_string());
        quest.due = Some("2026-03-01".to_string());
        insert_local_quest_row(&mut conn, &quest);

        let mut mark = QuestOccurrence::new(&quest.id, "2026-03-02", "2026-03-02T09:00:00Z");
        mark.status = Some(crate::models::OCCURRENCE_COMPLETED.to_string());
        mark.closed_at = Some("2026-03-02T09:00:00Z".to_string());
        let upsert = test_envelope(
            "evt-mark",
            "peer",
            "quest_occurrence",
            &mark.sync_id(),
            "upsert",
            Some(serde_json::to_string(&mark).unwrap()),
            "2026-03-02T09:00:00Z",
        );
        assert!(apply_sync_event(&mut conn, &upsert).unwrap());
        let stored: Vec<(String, Option<String>)> = quest_occurrences::table
            .filter(quest_occurrences::quest_id.eq(&quest.id))
            .select((quest_occurrences::period_key, quest_occurrences::status))
            .load(&mut conn)
            .unwrap();
        assert_eq!(
            stored,
            vec![("2026-03-02".to_string(), Some("completed".to_string()))]
        );

        let delete = test_envelope(
            "evt-mark-delete",
            "peer",
            "quest_occurrence",
            &mark.sync_id(),
            "delete",
            None,
            "2026-03-02T10:00:00Z",
        );
        assert!(apply_sync_event(&mut conn, &delete).unwrap());
        let left: i64 = quest_occurrences::table
            .count()
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(left, 0);

        let _ = std::fs::remove_file(db_path);
    }

    fn insert_local_quest_row(conn: &mut SqliteConnection, quest: &Quest) {
        diesel::insert_into(quests::table)
            .values((
                quests::id.eq(&quest.id),
                quests::space_id.eq(&quest.space_id),
                quests::title.eq(&quest.title),
                quests::description.eq(&quest.description),
                quests::status.eq(&quest.status),
                quests::energy.eq(&quest.energy),
                quests::priority.eq(quest.priority),
                quests::due.eq(&quest.due),
                quests::due_time.eq(&quest.due_time),
                quests::repeat_rule.eq(&quest.repeat_rule),
                quests::completed_at.eq(&quest.completed_at),
                quests::order_rank.eq(&quest.order_rank),
                quests::focus_enter_count.eq(quest.focus_enter_count),
                quests::created_at.eq(&quest.created_at),
                quests::updated_at.eq(&quest.updated_at),
                quests::is_checklist.eq(quest.is_checklist),
                quests::checklist_base.eq(&quest.checklist_base),
            ))
            .execute(conn)
            .unwrap();
    }

    /// Issue #128's core guarantee: independent packing actions on two different checklist items,
    /// made on two devices from the same base, must both survive a sync exchange — even though
    /// the enclosing quest sync event necessarily has to pick ONE winner for non-checklist fields
    /// under whole-entity LWW.
    #[test]
    fn apply_sync_event_merges_independent_checklist_items_when_whole_entity_wins() {
        let db_path = temp_db_path("checklist-merge-win");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let base_checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        // Local device packed headphones (item a1) after the shared base.
        let local_checklist = crate::services::checklist::set_checked(&base_checklist, "a1", true);

        let mut local_quest = test_quest("q1", "Go to office", "2026-03-01T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(local_checklist);
        local_quest.checklist_base = Some(base_checklist.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        // Remote device independently packed the key fob (item a2) after the same base, and its
        // event is newer, so it wins the whole-entity comparison.
        let remote_checklist = crate::services::checklist::set_checked(&base_checklist, "a2", true);
        let mut remote_quest = test_quest("q1", "Go to office", "2026-03-02T00:00:00Z");
        remote_quest.is_checklist = true;
        remote_quest.description = Some(remote_checklist);
        let event = test_envelope(
            "evt-remote",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&remote_quest)),
            "2026-03-02T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let merged: Quest = quests::table
            .find("q1")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        let items = crate::services::checklist::parse(merged.description.as_deref().unwrap());
        assert!(
            items.iter().find(|it| it.id == "a1").unwrap().checked,
            "local's independent pack of item a1 must survive"
        );
        assert!(
            items.iter().find(|it| it.id == "a2").unwrap().checked,
            "remote's independent pack of item a2 must survive"
        );
        assert_eq!(
            merged.checklist_base.as_deref(),
            Some(base_checklist.as_str()),
            "merging local-only content must not advance the shared checklist base before the peer adopts it"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_preserves_local_checklist_mode_for_legacy_payloads() {
        let db_path = temp_db_path("legacy-checklist-mode");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let checklist =
            crate::services::checklist::serialize(&[crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            }]);

        let mut local_quest = test_quest("q-legacy", "Pack", "2026-03-01T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(checklist.clone());
        local_quest.checklist_base = Some(checklist.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let mut remote_quest = test_quest("q-legacy", "Pack for office", "2026-03-02T00:00:00Z");
        remote_quest.description = Some(checklist.clone());
        let mut legacy_payload = serde_json::to_value(&remote_quest).unwrap();
        legacy_payload
            .as_object_mut()
            .unwrap()
            .remove("is_checklist");
        let event = test_envelope(
            "evt-legacy-checklist",
            "dev-remote",
            "quest",
            "q-legacy",
            "upsert",
            Some(serde_json::to_string(&legacy_payload).unwrap()),
            "2026-03-02T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let after: Quest = quests::table
            .find("q-legacy")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(after.title, "Pack for office");
        assert_eq!(after.description.as_deref(), Some(checklist.as_str()));
        assert!(
            after.is_checklist,
            "legacy payloads without is_checklist must not demote locally-known checklists"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_seeds_checklist_base_for_new_synced_checklist() {
        let db_path = temp_db_path("synced-checklist-base");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);

        let mut remote_quest = test_quest("q-synced-checklist", "Pack", "2026-03-02T00:00:00Z");
        remote_quest.is_checklist = true;
        remote_quest.description = Some(checklist.clone());
        let event = test_envelope(
            "evt-synced-checklist",
            "dev-remote",
            "quest",
            "q-synced-checklist",
            "upsert",
            Some(quest_payload(&remote_quest)),
            "2026-03-02T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let stored: Quest = quests::table
            .find("q-synced-checklist")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        assert!(stored.is_checklist);
        assert_eq!(stored.description.as_deref(), Some(checklist.as_str()));
        assert_eq!(
            stored.checklist_base.as_deref(),
            Some(checklist.as_str()),
            "a first synced checklist insert must establish the shared base for later negative edits"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_seeds_checklist_base_when_existing_row_adopts_checklist_mode() {
        let db_path = temp_db_path("synced-checklist-conversion-base");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);

        let mut local_quest = test_quest("q-converted-checklist", "Pack", "2026-03-01T00:00:00Z");
        local_quest.description = Some(checklist.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let mut remote_quest = local_quest.clone();
        remote_quest.is_checklist = true;
        remote_quest.description = Some(checklist.clone());
        remote_quest.updated_at = "2026-03-02T00:00:00Z".to_string();
        let event = test_envelope(
            "evt-synced-checklist-conversion",
            "dev-remote",
            "quest",
            "q-converted-checklist",
            "upsert",
            Some(quest_payload(&remote_quest)),
            "2026-03-02T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let stored: Quest = quests::table
            .find("q-converted-checklist")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        assert!(stored.is_checklist);
        assert_eq!(stored.description.as_deref(), Some(checklist.as_str()));
        assert_eq!(
            stored.checklist_base.as_deref(),
            Some(checklist.as_str()),
            "a synced conversion to checklist mode must seed the shared base for later negative edits"
        );

        let _ = std::fs::remove_file(db_path);
    }

    /// Same guarantee, but for the case where the incoming event LOSES the whole-entity LWW
    /// comparison (e.g. it's older) — the checklist merge must still apply, because #128 forbids
    /// letting whole-quest LWW silently discard an independent packing action.
    #[test]
    fn apply_sync_event_merges_checklist_even_when_whole_entity_event_loses() {
        let db_path = temp_db_path("checklist-merge-lose");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let base_checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let local_checklist = crate::services::checklist::set_checked(&base_checklist, "a1", true);

        // Local's last-known event is newer than the incoming one, so the incoming event loses
        // the whole-entity comparison (title etc. must NOT be adopted from it).
        insert_outbox_entry(
            &mut conn,
            "dev-local",
            "quest",
            "q1",
            "2026-03-05T00:00:00Z",
            None,
        );
        let mut local_quest = test_quest("q1", "Go to office", "2026-03-05T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(local_checklist);
        local_quest.checklist_base = Some(base_checklist.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let remote_checklist = crate::services::checklist::set_checked(&base_checklist, "a2", true);
        let mut remote_quest = test_quest("q1", "Stale remote title", "2026-03-01T00:00:00Z");
        remote_quest.is_checklist = true;
        remote_quest.description = Some(remote_checklist);
        let event = test_envelope(
            "evt-remote-old",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&remote_quest)),
            "2026-03-01T00:00:00Z",
        );

        assert!(
            !apply_sync_event(&mut conn, &event).unwrap(),
            "the whole-entity event itself should report as not applied (it lost LWW)"
        );

        let after: Quest = quests::table
            .find("q1")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(
            after.title, "Go to office",
            "losing event must not overwrite other fields"
        );
        let items = crate::services::checklist::parse(after.description.as_deref().unwrap());
        assert!(
            items.iter().find(|it| it.id == "a1").unwrap().checked,
            "local's own pack of item a1 must remain"
        );
        assert!(
            items.iter().find(|it| it.id == "a2").unwrap().checked,
            "remote's independent pack of item a2 must still merge in, despite losing whole-entity LWW"
        );

        let _ = std::fs::remove_file(db_path);
    }

    /// ADR-0012 D5: two devices tick different items of the same day of a
    /// repeating checklist quest. The device whose own event is newer keeps
    /// both ticks and sends the merged day back, so both devices agree.
    #[test]
    fn apply_sync_event_merges_occurrence_checklist_when_its_event_loses() {
        let db_path = temp_db_path("occurrence-checklist-merge-lose");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();
        crate::services::settings::upsert_setting(&mut conn, "device.id", "dev-local").unwrap();

        let base_checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let mut quest = test_quest("q1", "Go to office", "2026-03-01T00:00:00Z");
        quest.is_checklist = true;
        quest.description = Some(base_checklist.clone());
        quest.repeat_rule = Some(r#"{"preset":"daily"}"#.to_string());
        quest.due = Some("2026-03-01".to_string());
        insert_local_quest_row(&mut conn, &quest);

        let mut local_mark = QuestOccurrence::new("q1", "2026-03-02", "2026-03-02T09:00:00Z");
        local_mark.description = Some(crate::services::checklist::set_checked(
            &base_checklist,
            "a1",
            true,
        ));
        local_mark.checklist_base = Some(base_checklist.clone());
        local_mark.updated_at = "2026-03-02T09:05:00Z".to_string();
        QuestRepository::new(&mut conn)
            .save_occurrence(&local_mark)
            .unwrap();
        insert_outbox_entry(
            &mut conn,
            "dev-local",
            "quest_occurrence",
            &local_mark.sync_id(),
            "2026-03-02T09:05:00Z",
            None,
        );

        let mut remote_mark = QuestOccurrence::new("q1", "2026-03-02", "2026-03-02T09:00:00Z");
        remote_mark.description = Some(crate::services::checklist::set_checked(
            &base_checklist,
            "a2",
            true,
        ));
        remote_mark.updated_at = "2026-03-02T09:01:00Z".to_string();
        let event = test_envelope(
            "evt-remote-mark",
            "dev-remote",
            "quest_occurrence",
            &remote_mark.sync_id(),
            "upsert",
            Some(serde_json::to_string(&remote_mark).unwrap()),
            "2026-03-02T09:01:00Z",
        );

        assert!(
            !apply_sync_event(&mut conn, &event).unwrap(),
            "the older event loses"
        );

        let after = QuestRepository::new(&mut conn)
            .find_occurrence("q1", "2026-03-02")
            .unwrap()
            .unwrap();
        let items = crate::services::checklist::parse(after.description.as_deref().unwrap());
        assert!(
            items.iter().all(|item| item.checked),
            "both ticks are kept: {items:?}"
        );

        let sent: Vec<Option<String>> = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest_occurrence"))
            .filter(sync_outbox::origin_device_id.eq("dev-local"))
            .filter(sync_outbox::payload.is_not_null())
            .select(sync_outbox::payload)
            .load(&mut conn)
            .unwrap();
        assert_eq!(sent.len(), 1, "the merged day goes back to the peer");
        let sent: QuestOccurrence = serde_json::from_str(sent[0].as_deref().unwrap()).unwrap();
        assert_eq!(sent.description, after.description);

        let _ = std::fs::remove_file(db_path);
    }

    /// A "This day" description of a quest that is not a checklist is prose:
    /// the newer edit wins whole, with no checklist merge that drops it.
    #[test]
    fn apply_sync_event_keeps_a_prose_day_description() {
        let db_path = temp_db_path("occurrence-prose-description");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();
        let mut quest = test_quest("q1", "Read", "2026-03-01T00:00:00Z");
        quest.repeat_rule = Some(r#"{"preset":"daily"}"#.to_string());
        quest.due = Some("2026-03-01".to_string());
        insert_local_quest_row(&mut conn, &quest);

        let mut local_mark = QuestOccurrence::new("q1", "2026-03-02", "2026-03-02T09:00:00Z");
        local_mark.description = Some("Chapter one".to_string());
        QuestRepository::new(&mut conn)
            .save_occurrence(&local_mark)
            .unwrap();

        let mut remote_mark = local_mark;
        remote_mark.description = Some("Chapter two".to_string());
        remote_mark.updated_at = "2026-03-02T10:00:00Z".to_string();
        let event = test_envelope(
            "evt-prose",
            "dev-remote",
            "quest_occurrence",
            &remote_mark.sync_id(),
            "upsert",
            Some(serde_json::to_string(&remote_mark).unwrap()),
            "2026-03-02T10:00:00Z",
        );
        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let after = QuestRepository::new(&mut conn)
            .find_occurrence("q1", "2026-03-02")
            .unwrap()
            .unwrap();
        assert_eq!(after.description.as_deref(), Some("Chapter two"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_emits_convergence_event_when_checklist_base_advances_only() {
        let db_path = temp_db_path("checklist-base-ack");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();
        crate::services::settings::upsert_setting(&mut conn, "device.id", "dev-local")
            .expect("seed local device id");

        let base_checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let adopted_checklist =
            crate::services::checklist::set_checked(&base_checklist, "a1", true);

        let mut local_quest = test_quest("q1", "Go to office", "2026-03-02T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(adopted_checklist.clone());
        local_quest.checklist_base = Some(base_checklist);
        insert_local_quest_row(&mut conn, &local_quest);

        let mut incoming_quest = test_quest("q1", "Go to office", "2026-03-02T00:00:00Z");
        incoming_quest.is_checklist = true;
        incoming_quest.description = Some(adopted_checklist.clone());
        let event = test_envelope(
            "evt-clean-adoption",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&incoming_quest)),
            "2026-03-02T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let after: Quest = quests::table
            .find("q1")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(
            after.description.as_deref(),
            Some(adopted_checklist.as_str())
        );
        assert_eq!(
            after.checklist_base.as_deref(),
            Some(adopted_checklist.as_str())
        );

        let emitted: Vec<(String, String, String, Option<String>)> = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest"))
            .filter(sync_outbox::entity_id.eq("q1"))
            .select((
                sync_outbox::origin_device_id,
                sync_outbox::op_type,
                sync_outbox::space_id,
                sync_outbox::payload,
            ))
            .load(&mut conn)
            .unwrap();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].0, "dev-local");
        assert_eq!(emitted[0].1, "upsert");
        assert_eq!(emitted[0].2, "1");
        let payload: Quest = serde_json::from_str(emitted[0].3.as_deref().unwrap()).unwrap();
        assert_eq!(
            payload.description.as_deref(),
            Some(adopted_checklist.as_str())
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_convergence_payload_keeps_winning_quest_metadata() {
        let db_path = temp_db_path("checklist-convergence-winning-metadata");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string(), "2".to_string()]).unwrap();
        crate::services::settings::upsert_setting(&mut conn, "device.id", "dev-local")
            .expect("seed local device id");

        let base_checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let local_checklist = crate::services::checklist::set_checked(&base_checklist, "a1", true);

        let mut local_quest = test_quest("q1", "Local stale title", "2026-03-01T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(local_checklist);
        local_quest.checklist_base = Some(base_checklist.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let remote_checklist = crate::services::checklist::set_checked(&base_checklist, "a2", true);
        let mut remote_quest = test_quest("q1", "Remote winning title", "2026-03-02T00:00:00Z");
        remote_quest.space_id = "2".to_string();
        remote_quest.status = "completed".to_string();
        remote_quest.completed_at = Some("2026-03-02T00:00:00Z".to_string());
        remote_quest.is_checklist = true;
        remote_quest.description = Some(remote_checklist);
        let event = test_envelope(
            "evt-winning-metadata",
            "dev-remote",
            "quest",
            "q1",
            "upsert",
            Some(quest_payload(&remote_quest)),
            "2026-03-02T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let stored: Quest = quests::table
            .find("q1")
            .select(Quest::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(stored.title, "Remote winning title");
        assert_eq!(stored.space_id, "2");
        assert_eq!(stored.status, "completed");

        let (emitted_space_id, emitted_payload): (String, Option<String>) = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest"))
            .filter(sync_outbox::entity_id.eq("q1"))
            .select((sync_outbox::space_id, sync_outbox::payload))
            .first(&mut conn)
            .unwrap();
        assert_eq!(emitted_space_id, "2");
        let emitted: Quest = serde_json::from_str(
            emitted_payload
                .as_deref()
                .expect("convergence payload should be present"),
        )
        .unwrap();
        assert_eq!(emitted.title, "Remote winning title");
        assert_eq!(emitted.space_id, "2");
        assert_eq!(emitted.status, "completed");
        assert_eq!(
            emitted.completed_at.as_deref(),
            Some("2026-03-02T00:00:00Z")
        );

        let emitted_items =
            crate::services::checklist::parse(emitted.description.as_deref().unwrap());
        assert!(
            emitted_items
                .iter()
                .find(|it| it.id == "a1")
                .unwrap()
                .checked
        );
        assert!(
            emitted_items
                .iter()
                .find(|it| it.id == "a2")
                .unwrap()
                .checked
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_convergence_uses_current_space_when_whole_entity_loses() {
        let db_path = temp_db_path("checklist-convergence-current-space");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string(), "2".to_string()]).unwrap();
        crate::services::settings::upsert_setting(&mut conn, "device.id", "dev-local")
            .expect("seed local device id");

        let base_checklist = crate::services::checklist::serialize(&[
            crate::services::checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let local_checklist = crate::services::checklist::set_checked(&base_checklist, "a1", true);

        insert_outbox_entry_for_space(
            &mut conn,
            "dev-local",
            "quest",
            "q-current-space",
            "2",
            "2026-03-05T00:00:00Z",
            None,
        );
        let mut local_quest =
            test_quest("q-current-space", "Moved locally", "2026-03-05T00:00:00Z");
        local_quest.space_id = "2".to_string();
        local_quest.is_checklist = true;
        local_quest.description = Some(local_checklist);
        local_quest.checklist_base = Some(base_checklist.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let remote_checklist = crate::services::checklist::set_checked(&base_checklist, "a2", true);
        let mut remote_quest = test_quest(
            "q-current-space",
            "Remote old-space edit",
            "2026-03-01T00:00:00Z",
        );
        remote_quest.space_id = "1".to_string();
        remote_quest.is_checklist = true;
        remote_quest.description = Some(remote_checklist);
        let event = test_envelope(
            "evt-old-space-checklist-edit",
            "dev-remote",
            "quest",
            "q-current-space",
            "upsert",
            Some(quest_payload(&remote_quest)),
            "2026-03-01T00:00:00Z",
        );

        assert!(
            !apply_sync_event(&mut conn, &event).unwrap(),
            "the old-space whole-quest event should lose to the newer local move"
        );

        let emitted: (String, Option<String>) = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest"))
            .filter(sync_outbox::entity_id.eq("q-current-space"))
            .filter(sync_outbox::payload.is_not_null())
            .select((sync_outbox::space_id, sync_outbox::payload))
            .first(&mut conn)
            .unwrap();
        assert_eq!(
            emitted.0, "2",
            "convergence event must be routable through the quest's current space"
        );
        let payload: Quest = serde_json::from_str(emitted.1.as_deref().unwrap()).unwrap();
        assert_eq!(payload.space_id, "2");
        let items = crate::services::checklist::parse(payload.description.as_deref().unwrap());
        assert!(items.iter().find(|it| it.id == "a1").unwrap().checked);
        assert!(items.iter().find(|it| it.id == "a2").unwrap().checked);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_updates_last_synced_for_origin_peer_space() {
        let db_path = temp_db_path("apply-sync-event-updates-last-synced");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();
        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("dev-remote"),
                paired_devices::display_name.eq("Remote"),
                paired_devices::paired_at.eq("2026-03-03T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(&mut conn)
            .unwrap();

        diesel::insert_into(pair_space_mappings::table)
            .values((
                pair_space_mappings::peer_device_id.eq("dev-remote"),
                pair_space_mappings::space_id.eq("1"),
                pair_space_mappings::enabled_at.eq("2026-03-03T00:00:00Z"),
                pair_space_mappings::last_synced_at.eq(Option::<String>::None),
            ))
            .execute(&mut conn)
            .unwrap();

        let quest = test_quest("q-sync", "Remote Quest", "2026-03-03T00:00:00Z");
        let event = test_envelope(
            "evt-sync",
            "dev-remote",
            "quest",
            "q-sync",
            "upsert",
            Some(quest_payload(&quest)),
            "2026-03-03T00:00:00Z",
        );

        assert!(apply_sync_event(&mut conn, &event).unwrap());

        let last_synced: Option<String> = pair_space_mappings::table
            .filter(pair_space_mappings::peer_device_id.eq("dev-remote"))
            .filter(pair_space_mappings::space_id.eq("1"))
            .select(pair_space_mappings::last_synced_at)
            .first(&mut conn)
            .unwrap();
        assert_eq!(last_synced.as_deref(), Some("2026-03-03T00:00:00Z"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn delete_entity_creates_tombstone_and_removes_row() {
        let db_path = temp_db_path("delete-tombstone");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let quest = test_quest("q-del", "To Delete", "2026-03-01T00:00:00Z");
        let upsert = test_envelope(
            "evt-create",
            "dev-remote",
            "quest",
            "q-del",
            "upsert",
            Some(quest_payload(&quest)),
            "2026-03-01T00:00:00Z",
        );
        apply_sync_event(&mut conn, &upsert).unwrap();

        let count: i64 = quests::table
            .filter(quests::id.eq("q-del"))
            .count()
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(count, 1);

        let del = test_envelope(
            "evt-del",
            "dev-remote",
            "quest",
            "q-del",
            "delete",
            None,
            "2026-03-02T00:00:00Z",
        );
        apply_sync_event(&mut conn, &del).unwrap();

        let count: i64 = quests::table
            .filter(quests::id.eq("q-del"))
            .count()
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(count, 0);

        let tombstone_count: i64 = tombstones::table
            .filter(tombstones::entity_id.eq("q-del"))
            .count()
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(tombstone_count, 1);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn cleanup_old_tombstones_removes_expired() {
        let db_path = temp_db_path("tombstone-cleanup");
        let mut conn = open_db_at_path(&db_path);

        let old_date = (chrono::Utc::now() - chrono::Duration::days(31))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        diesel::insert_into(tombstones::table)
            .values((
                tombstones::entity_type.eq("quest"),
                tombstones::entity_id.eq("old-q"),
                tombstones::space_id.eq("1"),
                tombstones::deleted_at.eq(&old_date),
            ))
            .execute(&mut conn)
            .unwrap();

        diesel::insert_into(tombstones::table)
            .values((
                tombstones::entity_type.eq("quest"),
                tombstones::entity_id.eq("recent-q"),
                tombstones::space_id.eq("1"),
                tombstones::deleted_at.eq(utc_now()),
            ))
            .execute(&mut conn)
            .unwrap();

        assert_eq!(cleanup_old_tombstones(&mut conn).unwrap(), 1);

        let remaining: i64 = tombstones::table.count().get_result(&mut conn).unwrap();
        assert_eq!(remaining, 1);

        let _ = std::fs::remove_file(db_path);
    }
}
