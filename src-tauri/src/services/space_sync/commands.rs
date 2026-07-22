use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
#[cfg(any(feature = "ui-plane", test))]
use tauri::State;

use super::merge::incoming_wins;
use super::outbox::{load_latest_event_for_entity, load_unacked_events_for_peer};
use super::replay::{is_event_seen, mark_event_seen, record_ack};
use super::types::{SyncEventEnvelope, WsMessage};
use super::ws_client::ensure_peer_sessions;
use crate::models::{
    ChecklistActivity, CreatePairSpaceMappingInput, FocusHistoryEntry, Quest, QuestSeries, Space,
};
use crate::schema::{
    checklist_activity, focus_history, pair_space_mappings, paired_devices, quest_series, quests,
    spaces, sync_acks, sync_outbox, sync_seen, tombstones,
};
use crate::services::db::utc_now;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
use crate::services::device_connection::{CustomSpaceDescriptor, DeviceConnectionState};

const MAX_EVENTS_PER_PEER_PER_TICK: usize = 64;

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncStatus {
    pub peer_device_id: Option<String>,
    pub mapped_space_ids: Vec<String>,
    pub last_synced_at: Option<String>,
    pub last_synced_at_by_space: BTreeMap<String, Option<String>>,
    pub end_of_sync_at_by_space: BTreeMap<String, Option<String>>,
    pub pending_event_count: usize,
    pub outbox_event_count: i64,
    pub acked_event_count: i64,
    pub seen_event_count: i64,
    pub tombstone_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncTickPeer {
    pub peer_device_id: String,
    pub sent_events: usize,
    pub received_events: usize,
    pub acked_events: usize,
    pub transferred_objects: usize,
    pub last_transfer_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncTickResult {
    pub sent_events: usize,
    pub applied_events: usize,
    pub received_acks: usize,
    pub peers: Vec<SpaceSyncTickPeer>,
    pub ticked_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedCustomSpace {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceMappingApplyResult {
    pub mapped_space_ids: Vec<String>,
    pub unresolved_custom_spaces: Vec<UnresolvedCustomSpace>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceResolutionMode {
    CreateNew,
    UseExisting,
}

fn normalized_space_ids(mapped_space_ids: Vec<String>) -> Vec<String> {
    mapped_space_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn is_builtin_space_id(space_id: &str) -> bool {
    matches!(space_id, "1" | "2" | "3")
}

fn placeholder_space_name(space_id: &str) -> String {
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

    let mut next_order = spaces::table
        .select(diesel::dsl::max(spaces::item_order))
        .first::<Option<i64>>(conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
        + 1;

    for space_id in missing {
        diesel::insert_or_ignore_into(spaces::table)
            .values((
                spaces::id.eq(&space_id),
                spaces::name.eq(placeholder_space_name(&space_id)),
                spaces::item_order.eq(next_order),
            ))
            .execute(conn)
            .map_err(|e| e.to_string())?;
        next_order += 1;
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

fn load_custom_space_descriptors(
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

fn list_mappings_for_peer(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
) -> Result<Vec<String>, String> {
    pair_space_mappings::table
        .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
        .filter(pair_space_mappings::end_of_sync_at.is_null())
        .order(pair_space_mappings::space_id.asc())
        .select(pair_space_mappings::space_id)
        .load(conn)
        .map_err(|e| e.to_string())
}

fn list_all_mapping_ids_for_peer(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
) -> Result<Vec<String>, String> {
    pair_space_mappings::table
        .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
        .order(pair_space_mappings::space_id.asc())
        .select(pair_space_mappings::space_id)
        .load(conn)
        .map_err(|e| e.to_string())
}

fn mark_mapping_ended(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    space_id: &str,
    ended_at: &str,
) -> Result<(), String> {
    diesel::update(
        pair_space_mappings::table
            .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
            .filter(pair_space_mappings::space_id.eq(space_id)),
    )
    .set(pair_space_mappings::end_of_sync_at.eq(Some(ended_at.to_string())))
    .execute(conn)
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn peer_is_paired(conn: &mut SqliteConnection, peer_device_id: &str) -> Result<bool, String> {
    let count: i64 = paired_devices::table
        .find(peer_device_id)
        .count()
        .get_result(conn)
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

fn apply_mappings_in_db(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if !peer_is_paired(conn, peer_device_id)? {
        return Err("paired device not found".to_string());
    }

    let desired_ids = normalized_space_ids(mapped_space_ids);
    let missing = missing_space_ids(conn, &desired_ids)?;
    if !missing.is_empty() {
        return Err(format!("unknown space ids: {}", missing.join(", ")));
    }

    let existing_ids = list_mappings_for_peer(conn, peer_device_id)?;
    let all_existing_ids = list_all_mapping_ids_for_peer(conn, peer_device_id)?;
    let existing_set: BTreeSet<String> = existing_ids.into_iter().collect();
    let all_existing_set: BTreeSet<String> = all_existing_ids.into_iter().collect();
    let desired_set: BTreeSet<String> = desired_ids.iter().cloned().collect();

    let to_add: Vec<String> = desired_set.difference(&existing_set).cloned().collect();
    let to_remove: Vec<String> = existing_set.difference(&desired_set).cloned().collect();

    if !to_remove.is_empty() {
        let ended_at = utc_now();
        diesel::update(
            pair_space_mappings::table
                .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
                .filter(pair_space_mappings::space_id.eq_any(&to_remove)),
        )
        .set(pair_space_mappings::end_of_sync_at.eq(Some(ended_at)))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    }

    let to_reenable: Vec<String> = to_add
        .iter()
        .filter(|space_id| all_existing_set.contains(*space_id))
        .cloned()
        .collect();
    let to_insert: Vec<String> = to_add
        .iter()
        .filter(|space_id| !all_existing_set.contains(*space_id))
        .cloned()
        .collect();

    if !to_reenable.is_empty() {
        let now = utc_now();
        diesel::update(
            pair_space_mappings::table
                .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
                .filter(pair_space_mappings::space_id.eq_any(&to_reenable)),
        )
        .set((
            pair_space_mappings::enabled_at.eq(now),
            pair_space_mappings::last_synced_at.eq(Option::<String>::None),
            pair_space_mappings::end_of_sync_at.eq(Option::<String>::None),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    }

    if !to_insert.is_empty() {
        let now = utc_now();
        let rows: Vec<CreatePairSpaceMappingInput> = to_add
            .iter()
            .filter(|space_id| !all_existing_set.contains(*space_id))
            .map(|space_id| CreatePairSpaceMappingInput {
                peer_device_id: peer_device_id.to_string(),
                space_id: space_id.clone(),
                enabled_at: now.clone(),
                last_synced_at: None,
                end_of_sync_at: None,
            })
            .collect();

        diesel::insert_or_ignore_into(pair_space_mappings::table)
            .values(&rows)
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    list_mappings_for_peer(conn, peer_device_id)
}

fn send_mapping_update_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    mapped_space_ids: &[String],
    custom_spaces: &[CustomSpaceDescriptor],
) -> Result<(), String> {
    if device_connection.push_to_peer(
        peer_device_id,
        WsMessage::SpaceMappingUpdate {
            mapped_space_ids: mapped_space_ids.to_vec(),
            custom_spaces: custom_spaces.to_vec(),
            sent_at: utc_now(),
        },
    ) {
        Ok(())
    } else {
        Err(format!("no active session for peer {peer_device_id}"))
    }
}

fn send_space_sync_end_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    space_id: &str,
    ended_at: &str,
) -> Result<(), String> {
    if device_connection.push_to_peer(
        peer_device_id,
        WsMessage::SpaceSyncEnd {
            space_id: space_id.to_string(),
            ended_at: ended_at.to_string(),
        },
    ) {
        Ok(())
    } else {
        Err(format!("no active session for peer {peer_device_id}"))
    }
}

fn send_sync_event_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    event: &SyncEventEnvelope,
) -> Result<(), String> {
    if device_connection.push_to_peer(peer_device_id, WsMessage::SyncEvent(event.clone())) {
        Ok(())
    } else {
        Err(format!("no active session for peer {peer_device_id}"))
    }
}

fn send_sync_ack_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    event_id: &str,
) -> Result<(), String> {
    if device_connection.push_to_peer(
        peer_device_id,
        WsMessage::Ack {
            event_id: event_id.to_string(),
        },
    ) {
        Ok(())
    } else {
        Err(format!("no active session for peer {peer_device_id}"))
    }
}

fn peer_has_mapping_for_space(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    space_id: &str,
) -> Result<bool, String> {
    let count: i64 = pair_space_mappings::table
        .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
        .filter(pair_space_mappings::space_id.eq(space_id))
        .filter(pair_space_mappings::end_of_sync_at.is_null())
        .count()
        .get_result(conn)
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
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
    ensure_spaces_exist(conn, &[quest.space_id.clone()])?;

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
        ))
        .on_conflict(quests::id)
        .do_update()
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
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn upsert_quest_series(conn: &mut SqliteConnection, series: &QuestSeries) -> Result<(), String> {
    ensure_spaces_exist(conn, &[series.space_id.clone()])?;

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
        .on_conflict(quest_series::id)
        .do_update()
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
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn upsert_focus_history(
    conn: &mut SqliteConnection,
    focus: &FocusHistoryEntry,
) -> Result<(), String> {
    ensure_spaces_exist(conn, &[focus.space_id.clone()])?;

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
            diesel::delete(quests::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        "quest_series" => {
            diesel::delete(quest_series::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
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

fn cleanup_old_tombstones(conn: &mut SqliteConnection) -> Result<usize, String> {
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
/// `device_connection::runtime::DEVICE_ID_KEY`) so the checklist merge push-back can stamp an
/// accurate `origin_device_id` without needing app-state (`apply_sync_event` only has `conn`).
fn local_device_id(conn: &mut SqliteConnection) -> Option<String> {
    crate::services::settings::load_setting(conn, "device.id")
        .ok()
        .flatten()
}

/// Merges an incoming checklist quest's `description` (task-list markdown) against local state
/// and persists the result, regardless of whether the incoming event wins the whole-entity LWW
/// comparison for the quest's other fields. Issue #128 explicitly forbids whole-quest LWW for
/// checklist item state — this is the retained-base 3-way merge from the storage spike
/// (`services::checklist_md::merge_3way`). Only called when the local quest is a checklist quest
/// (`is_checklist`); plain prose quests never enter this path and follow normal whole-entity LWW
/// on `description` like any other field. Pushes the merged result back out to the origin peer
/// when it differs from what they sent, so both sides converge. Returns the merged description
/// for the caller to fold into `upsert_quest`.
fn merge_and_persist_quest_checklist(
    conn: &mut SqliteConnection,
    remote_origin_device_id: &str,
    entity_id: &str,
    entity_space_id: &str,
    incoming_description: Option<&str>,
) -> Result<Option<String>, String> {
    let local = quests::table
        .find(entity_id)
        .select(Quest::as_select())
        .first::<Quest>(conn)
        .optional()
        .map_err(|e| e.to_string())?;

    let local = match local {
        // Brand-new quest arriving via sync: nothing to merge against yet, adopt incoming as-is.
        None => return Ok(incoming_description.map(|s| s.to_string())),
        Some(local) => local,
    };

    let (merged_raw, _had_text_conflict) = crate::services::checklist_md::merge_3way(
        local.checklist_base.as_deref(),
        local.description.as_deref().unwrap_or(""),
        incoming_description.unwrap_or(""),
    );
    let merged = if merged_raw.is_empty() {
        None
    } else {
        Some(merged_raw)
    };

    let checklist_base_before = local.checklist_base.clone();
    let incoming_matches_merged = incoming_description == merged.as_deref();
    let next_checklist_base = if incoming_matches_merged {
        merged.clone()
    } else {
        checklist_base_before.clone()
    };
    let base_advanced = checklist_base_before.as_deref() != next_checklist_base.as_deref();

    if merged != local.description {
        diesel::update(quests::table.find(entity_id))
            .set((
                quests::description.eq(&merged),
                quests::checklist_base.eq(&next_checklist_base),
            ))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    } else if base_advanced {
        diesel::update(quests::table.find(entity_id))
            .set(quests::checklist_base.eq(&next_checklist_base))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    // Merge changed something relative to what the remote peer sent, or advanced our retained
    // checklist base after cleanly adopting the peer's state. Push a convergence event back so
    // the origin can advance its own retained base too; otherwise a later negative edit from the
    // adopter can be merged against the origin's stale pre-edit base and be reverted.
    if merged.as_deref() != incoming_description || base_advanced {
        if let Some(own_device_id) = local_device_id(conn) {
            if own_device_id != remote_origin_device_id {
                if let Ok(refreshed) = quests::table
                    .find(entity_id)
                    .select(Quest::as_select())
                    .first::<Quest>(conn)
                {
                    if let Ok(payload) = serde_json::to_string(&refreshed) {
                        let _ = super::outbox::emit_sync_event(
                            conn,
                            &own_device_id,
                            "quest",
                            entity_id,
                            entity_space_id,
                            "upsert",
                            Some(payload),
                        );
                    }
                }
            }
        }
    }

    Ok(merged)
}

fn apply_sync_event(
    conn: &mut SqliteConnection,
    event: &SyncEventEnvelope,
) -> Result<bool, String> {
    if is_event_seen(conn, &event.event_id)? {
        return Ok(false);
    }

    match event.op_type.as_str() {
        "delete" => {
            delete_entity(conn, event)?;
        }
        "upsert" => {
            let whole_entity_wins =
                match load_latest_event_for_entity(conn, &event.entity_type, &event.entity_id)? {
                    Some(local_event) => incoming_wins(event, &local_event),
                    None => true,
                };

            let payload = event
                .payload
                .as_ref()
                .ok_or_else(|| "missing payload for upsert event".to_string())?;

            // Checklist items get a per-item merge instead of following whole-quest LWW (#128),
            // so a locally-known checklist quest is handled before the generic win/lose gate
            // below. A quest that isn't (yet) known locally as a checklist quest — including a
            // brand-new quest arriving for the first time — falls through to the normal
            // whole-entity path; its `description` is treated like any other field.
            let local_is_checklist = if event.entity_type == "quest" {
                quests::table
                    .find(&event.entity_id)
                    .select(quests::is_checklist)
                    .first::<bool>(conn)
                    .optional()
                    .map_err(|e| e.to_string())?
                    .unwrap_or(false)
            } else {
                false
            };

            if local_is_checklist {
                let mut quest: Quest = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                let merged_description = merge_and_persist_quest_checklist(
                    conn,
                    &event.origin_device_id,
                    &event.entity_id,
                    &event.space_id,
                    quest.description.as_deref(),
                )?;

                if whole_entity_wins {
                    quest.description = merged_description;
                    quest.is_checklist = true;
                    upsert_quest(conn, &quest)?;
                } else {
                    mark_event_seen(conn, &event.event_id)?;
                    return Ok(false);
                }
            } else if !whole_entity_wins {
                mark_event_seen(conn, &event.event_id)?;
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
                    "quest_series" => {
                        let series: QuestSeries =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_quest_series(conn, &series)?;
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

    mark_event_seen(conn, &event.event_id)?;

    let current_last_synced = pair_space_mappings::table
        .filter(pair_space_mappings::peer_device_id.eq(&event.origin_device_id))
        .filter(pair_space_mappings::space_id.eq(&event.space_id))
        .select(pair_space_mappings::last_synced_at)
        .first::<Option<String>>(conn)
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();

    if current_last_synced
        .as_deref()
        .map(|value| value < event.updated_at.as_str())
        .unwrap_or(true)
    {
        diesel::update(
            pair_space_mappings::table
                .filter(pair_space_mappings::peer_device_id.eq(&event.origin_device_id))
                .filter(pair_space_mappings::space_id.eq(&event.space_id)),
        )
        .set(pair_space_mappings::last_synced_at.eq(Some(event.updated_at.clone())))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    }

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

fn create_space_with_id(
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

fn merge_local_space_into_remote_id(
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

    diesel::update(quests::table.filter(quests::space_id.eq(local_space_id)))
        .set(quests::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    diesel::update(quest_series::table.filter(quest_series::space_id.eq(local_space_id)))
        .set(quest_series::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    diesel::update(focus_history::table.filter(focus_history::space_id.eq(local_space_id)))
        .set(focus_history::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    diesel::update(sync_outbox::table.filter(sync_outbox::space_id.eq(local_space_id)))
        .set(sync_outbox::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    let peers_with_old_mapping: Vec<String> = pair_space_mappings::table
        .filter(pair_space_mappings::space_id.eq(local_space_id))
        .select(pair_space_mappings::peer_device_id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    for peer_device_id in peers_with_old_mapping {
        diesel::insert_or_ignore_into(pair_space_mappings::table)
            .values((
                pair_space_mappings::peer_device_id.eq(&peer_device_id),
                pair_space_mappings::space_id.eq(remote_space_id),
                pair_space_mappings::enabled_at.eq(utc_now()),
            ))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    diesel::delete(
        pair_space_mappings::table.filter(pair_space_mappings::space_id.eq(local_space_id)),
    )
    .execute(conn)
    .map_err(|e| e.to_string())?;

    diesel::delete(spaces::table.find(local_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn partition_remote_mapped_spaces(
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

pub fn space_sync_list_mappings_impl(
    conn: &mut SqliteConnection,
    peer_device_id: String,
) -> Result<Vec<String>, String> {
    list_mappings_for_peer(conn, &peer_device_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn space_sync_list_mappings(
    db: State<AppDbConnection>,
    peer_device_id: String,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().unwrap();
    space_sync_list_mappings_impl(&mut conn, peer_device_id)
}

pub fn space_sync_update_mappings_impl(
    conn: &mut SqliteConnection,
    device_connection: &DeviceConnectionState,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let before = list_mappings_for_peer(conn, &peer_device_id)?;
    let before_set: BTreeSet<String> = before.into_iter().collect();
    let desired = normalized_space_ids(mapped_space_ids);
    let desired_set: BTreeSet<String> = desired.iter().cloned().collect();
    let requested: Vec<String> = desired_set.difference(&before_set).cloned().collect();
    let ended: Vec<String> = before_set.difference(&desired_set).cloned().collect();
    let ended_at = utc_now();

    let mapped = apply_mappings_in_db(conn, &peer_device_id, desired)?;

    let custom_spaces = load_custom_space_descriptors(conn, &requested)?;

    for space_id in requested {
        let custom = custom_spaces
            .iter()
            .filter(|space| space.space_id == space_id)
            .cloned()
            .collect::<Vec<_>>();
        if let Err(err) =
            send_mapping_update_to_peer(&device_connection, &peer_device_id, &[space_id], &custom)
        {
            eprintln!("[space-sync] failed to notify peer mapping request: {err}");
        }
    }

    for space_id in ended {
        if let Err(err) =
            send_space_sync_end_to_peer(&device_connection, &peer_device_id, &space_id, &ended_at)
        {
            eprintln!("[space-sync] failed to notify peer mapping end: {err}");
        }
    }

    Ok(mapped)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn space_sync_update_mappings(
    db: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().unwrap();
    space_sync_update_mappings_impl(
        &mut conn,
        &device_connection,
        peer_device_id,
        mapped_space_ids,
    )
}

pub fn space_sync_apply_remote_mappings_impl(
    conn: &mut SqliteConnection,
    _device_connection: &DeviceConnectionState,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<SpaceMappingApplyResult, String> {
    let (resolved_ids, unresolved_custom_spaces) =
        partition_remote_mapped_spaces(conn, mapped_space_ids, custom_spaces)?;

    let mut desired = list_mappings_for_peer(conn, &peer_device_id)?;
    for space_id in resolved_ids {
        if !desired.contains(&space_id) {
            desired.push(space_id);
        }
    }

    let mapped_space_ids = apply_mappings_in_db(conn, &peer_device_id, desired)?;

    Ok(SpaceMappingApplyResult {
        mapped_space_ids,
        unresolved_custom_spaces,
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn space_sync_apply_remote_mappings(
    db: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<SpaceMappingApplyResult, String> {
    let mut conn = db.0.lock().unwrap();
    space_sync_apply_remote_mappings_impl(
        &mut conn,
        &device_connection,
        peer_device_id,
        mapped_space_ids,
        custom_spaces,
    )
}

pub fn space_sync_resolve_custom_space_mapping_impl(
    conn: &mut SqliteConnection,
    device_connection: &DeviceConnectionState,
    peer_device_id: String,
    remote_space_id: String,
    remote_space_name: Option<String>,
    resolution_mode: SpaceResolutionMode,
    existing_space_id: Option<String>,
) -> Result<SpaceMappingApplyResult, String> {
    if is_builtin_space_id(&remote_space_id) {
        return Err("remote custom-space resolution expects non built-in space id".to_string());
    }

    match resolution_mode {
        SpaceResolutionMode::CreateNew => {
            let name = remote_space_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| placeholder_space_name(&remote_space_id));
            create_space_with_id(conn, &remote_space_id, &name)?;
        }
        SpaceResolutionMode::UseExisting => {
            let local_space_id = existing_space_id
                .as_deref()
                .ok_or_else(|| "existing_space_id is required for use_existing".to_string())?;
            merge_local_space_into_remote_id(
                conn,
                local_space_id,
                &remote_space_id,
                remote_space_name.as_deref(),
            )?;
        }
    }

    let mut mapped = list_mappings_for_peer(conn, &peer_device_id)?;
    if !mapped.contains(&remote_space_id) {
        mapped.push(remote_space_id.clone());
    }

    let mapped_space_ids = apply_mappings_in_db(conn, &peer_device_id, mapped)?;

    let custom_spaces = load_custom_space_descriptors(conn, &mapped_space_ids)?;
    if let Err(err) = send_mapping_update_to_peer(
        &device_connection,
        &peer_device_id,
        &mapped_space_ids,
        &custom_spaces,
    ) {
        eprintln!("[space-sync] failed to notify peer mapping resolve: {err}");
    }

    Ok(SpaceMappingApplyResult {
        mapped_space_ids,
        unresolved_custom_spaces: Vec::new(),
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn space_sync_resolve_custom_space_mapping(
    db: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    remote_space_id: String,
    remote_space_name: Option<String>,
    resolution_mode: SpaceResolutionMode,
    existing_space_id: Option<String>,
) -> Result<SpaceMappingApplyResult, String> {
    let mut conn = db.0.lock().unwrap();
    space_sync_resolve_custom_space_mapping_impl(
        &mut conn,
        &device_connection,
        peer_device_id,
        remote_space_id,
        remote_space_name,
        resolution_mode,
        existing_space_id,
    )
}

pub fn space_sync_tick_impl(
    mut conn: &mut SqliteConnection,
    device_connection: &DeviceConnectionState,
) -> Result<SpaceSyncTickResult, String> {
    let peer_ids: Vec<String> = paired_devices::table
        .select(paired_devices::peer_device_id)
        .load(&mut *conn)
        .map_err(|e| e.to_string())?;

    // Ensure WS sessions are open for peers where we are the dialer
    let paired_peer_ids: HashSet<String> = peer_ids.iter().cloned().collect();
    ensure_peer_sessions(
        device_connection,
        device_connection.db_path.clone(),
        &paired_peer_ids,
    );

    let ticked_at = utc_now();
    let mut transferred_by_peer: std::collections::HashMap<String, (usize, usize, usize)> =
        std::collections::HashMap::new();

    for ended in device_connection.take_incoming_space_sync_ends() {
        mark_mapping_ended(
            &mut conn,
            &ended.from_device_id,
            &ended.space_id,
            &ended.ended_at,
        )?;
    }

    let mut sent_events_total = 0usize;

    for peer_device_id in &peer_ids {
        let mapped = list_mappings_for_peer(&mut conn, &peer_device_id)?;
        let pending = load_unacked_events_for_peer(&mut conn, &peer_device_id, &mapped)?;
        if pending.is_empty() {
            continue;
        }

        let mut sent_events = 0usize;
        for event in pending.iter().take(MAX_EVENTS_PER_PEER_PER_TICK) {
            if send_sync_event_to_peer(&device_connection, &peer_device_id, event).is_ok() {
                sent_events += 1;
            }
        }

        sent_events_total += sent_events;
        if sent_events > 0 {
            let entry = transferred_by_peer
                .entry(peer_device_id.clone())
                .or_insert((0, 0, 0));
            entry.0 += sent_events;
        }
    }

    for peer_device_id in &peer_ids {
        if !device_connection.has_session(peer_device_id) {
            continue;
        }
        let unsynced: Vec<String> = pair_space_mappings::table
            .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
            .filter(pair_space_mappings::last_synced_at.is_null())
            .filter(pair_space_mappings::end_of_sync_at.is_null())
            .select(pair_space_mappings::space_id)
            .load(&mut *conn)
            .map_err(|e| e.to_string())?;
        for space_id in unsynced {
            device_connection.push_to_peer(peer_device_id, WsMessage::BootstrapStart { space_id });
        }
    }

    let incoming_events = device_connection.take_incoming_sync_events();
    let mut applied_events = 0usize;
    let mut deferred_events = Vec::new();
    for event in incoming_events {
        match peer_has_mapping_for_space(&mut conn, &event.origin_device_id, &event.space_id) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                deferred_events.push(event);
                continue;
            }
        }

        match apply_sync_event(&mut conn, &event) {
            Ok(applied) => {
                if applied {
                    applied_events += 1;
                    let entry = transferred_by_peer
                        .entry(event.origin_device_id.clone())
                        .or_insert((0, 0, 0));
                    entry.1 += 1;
                }
                let _ = send_sync_ack_to_peer(
                    &device_connection,
                    &event.origin_device_id,
                    &event.event_id,
                );
            }
            Err(err) => {
                eprintln!(
                    "[space-sync] failed to apply incoming event {}: {err}",
                    event.event_id
                );
            }
        }
    }
    if !deferred_events.is_empty() {
        device_connection.restore_incoming_sync_events(deferred_events);
    }

    let incoming_acks = device_connection.take_incoming_sync_acks();
    let mut received_acks = 0usize;
    for ack in incoming_acks {
        if let Ok(inserted) = record_ack(&mut conn, &ack.from_device_id, &ack.event_id) {
            if !inserted {
                continue;
            }

            let event_meta = sync_outbox::table
                .find(&ack.event_id)
                .select((sync_outbox::space_id, sync_outbox::updated_at))
                .first::<(String, String)>(&mut *conn)
                .optional()
                .map_err(|e| e.to_string())?;

            if let Some((space_id, event_updated_at)) = event_meta {
                let current_last_synced = pair_space_mappings::table
                    .filter(pair_space_mappings::peer_device_id.eq(&ack.from_device_id))
                    .filter(pair_space_mappings::space_id.eq(&space_id))
                    .select(pair_space_mappings::last_synced_at)
                    .first::<Option<String>>(&mut *conn)
                    .optional()
                    .map_err(|e| e.to_string())?
                    .flatten();

                if current_last_synced
                    .as_deref()
                    .map(|value| value < event_updated_at.as_str())
                    .unwrap_or(true)
                {
                    diesel::update(
                        pair_space_mappings::table
                            .filter(pair_space_mappings::peer_device_id.eq(&ack.from_device_id))
                            .filter(pair_space_mappings::space_id.eq(&space_id)),
                    )
                    .set(pair_space_mappings::last_synced_at.eq(Some(event_updated_at)))
                    .execute(&mut *conn)
                    .map_err(|e| e.to_string())?;
                }
            }

            received_acks += 1;
            let entry = transferred_by_peer
                .entry(ack.from_device_id.clone())
                .or_insert((0, 0, 0));
            entry.2 += 1;
        }
    }

    let mut peers: Vec<SpaceSyncTickPeer> = transferred_by_peer
        .into_iter()
        .map(
            |(peer_device_id, (sent_events, received_events, acked_events))| {
                let transferred_objects = received_events + acked_events;
                SpaceSyncTickPeer {
                    peer_device_id,
                    sent_events,
                    received_events,
                    acked_events,
                    transferred_objects,
                    last_transfer_at: ticked_at.clone(),
                }
            },
        )
        .collect();
    peers.sort_by(|a, b| a.peer_device_id.cmp(&b.peer_device_id));

    let _ = cleanup_old_tombstones(&mut conn);

    Ok(SpaceSyncTickResult {
        sent_events: sent_events_total,
        applied_events,
        received_acks,
        peers,
        ticked_at,
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn space_sync_tick(
    db: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
) -> Result<SpaceSyncTickResult, String> {
    let mut conn = db.0.lock().unwrap();
    space_sync_tick_impl(&mut conn, &device_connection)
}

pub fn space_sync_status_impl(
    mut conn: &mut SqliteConnection,
    peer_device_id: Option<String>,
) -> Result<SpaceSyncStatus, String> {
    let outbox_event_count: i64 = sync_outbox::table
        .count()
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())?;
    let seen_event_count: i64 = sync_seen::table
        .count()
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())?;
    let tombstone_count: i64 = tombstones::table
        .count()
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())?;

    let (
        mapped_space_ids,
        last_synced_at,
        last_synced_at_by_space,
        end_of_sync_at_by_space,
        pending_event_count,
        acked_event_count,
    ) = if let Some(peer_id) = peer_device_id.as_deref() {
        let mapping_rows: Vec<(String, Option<String>, Option<String>)> =
            pair_space_mappings::table
                .filter(pair_space_mappings::peer_device_id.eq(peer_id))
                .order(pair_space_mappings::space_id.asc())
                .select((
                    pair_space_mappings::space_id,
                    pair_space_mappings::last_synced_at,
                    pair_space_mappings::end_of_sync_at,
                ))
                .load(&mut *conn)
                .map_err(|e| e.to_string())?;

        let mapped: Vec<String> = mapping_rows
            .iter()
            .filter(|(_, _, ended_at)| ended_at.is_none())
            .map(|(space_id, _, _)| space_id.clone())
            .collect();
        let by_space: BTreeMap<String, Option<String>> = mapping_rows
            .iter()
            .filter(|(_, _, ended_at)| ended_at.is_none())
            .map(|(space_id, synced_at, _)| (space_id.clone(), synced_at.clone()))
            .collect();
        let ended_by_space: BTreeMap<String, Option<String>> = mapping_rows
            .into_iter()
            .filter(|(_, _, ended_at)| ended_at.is_some())
            .map(|(space_id, _, ended_at)| (space_id, ended_at))
            .collect();
        let last_synced = by_space.values().flatten().max().cloned();

        let pending = load_unacked_events_for_peer(&mut conn, peer_id, &mapped)?.len();
        let acked: i64 = sync_acks::table
            .filter(sync_acks::peer_device_id.eq(peer_id))
            .count()
            .get_result(&mut *conn)
            .map_err(|e| e.to_string())?;
        (
            mapped,
            last_synced,
            by_space,
            ended_by_space,
            pending,
            acked,
        )
    } else {
        (Vec::new(), None, BTreeMap::new(), BTreeMap::new(), 0, 0)
    };

    Ok(SpaceSyncStatus {
        peer_device_id,
        mapped_space_ids,
        last_synced_at,
        last_synced_at_by_space,
        end_of_sync_at_by_space,
        pending_event_count,
        outbox_event_count,
        acked_event_count,
        seen_event_count,
        tombstone_count,
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn space_sync_status(
    db: State<AppDbConnection>,
    peer_device_id: Option<String>,
) -> Result<SpaceSyncStatus, String> {
    let mut conn = db.0.lock().unwrap();
    space_sync_status_impl(&mut conn, peer_device_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateSyncOutboxEntry;
    use crate::services::db::open_db_at_path;
    use crate::services::device_connection::DeviceConnectionState;
    use std::path::PathBuf;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fini-test-cmd-{label}-{}.db", Uuid::new_v4()))
    }

    fn temp_app_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fini-test-app-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_quest(id: &str, title: &str, updated_at: &str) -> Quest {
        Quest {
            id: id.to_string(),
            space_id: "1".to_string(),
            title: title.to_string(),
            description: None,
            status: "active".to_string(),
            energy: "medium".to_string(),
            priority: 1,
            pinned: false,
            due: None,
            due_time: None,
            repeat_rule: None,
            completed_at: None,
            order_rank: 1000.0,
            focus_enter_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
            series_id: None,
            period_key: None,
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

        let base_md = crate::services::checklist_md::serialize(&[
            crate::services::checklist_md::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist_md::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        // Local device packed headphones (item a1) after the shared base.
        let local_md = crate::services::checklist_md::set_checked(&base_md, "a1", true);

        let mut local_quest = test_quest("q1", "Go to office", "2026-03-01T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(local_md.clone());
        local_quest.checklist_base = Some(base_md.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        // Remote device independently packed the key fob (item a2) after the same base, and its
        // event is newer, so it wins the whole-entity comparison.
        let remote_md = crate::services::checklist_md::set_checked(&base_md, "a2", true);
        let mut remote_quest = test_quest("q1", "Go to office", "2026-03-02T00:00:00Z");
        remote_quest.is_checklist = true;
        remote_quest.description = Some(remote_md);
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
        let items = crate::services::checklist_md::parse(merged.description.as_deref().unwrap());
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
            Some(base_md.as_str()),
            "merging local-only content must not advance the shared checklist base before the peer adopts it"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn apply_sync_event_preserves_local_checklist_mode_for_legacy_payloads() {
        let db_path = temp_db_path("legacy-checklist-mode");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let checklist_md = crate::services::checklist_md::serialize(&[
            crate::services::checklist_md::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
        ]);

        let mut local_quest = test_quest("q-legacy", "Pack", "2026-03-01T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(checklist_md.clone());
        local_quest.checklist_base = Some(checklist_md.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let mut remote_quest = test_quest("q-legacy", "Pack for office", "2026-03-02T00:00:00Z");
        remote_quest.description = Some(checklist_md.clone());
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
        assert_eq!(after.description.as_deref(), Some(checklist_md.as_str()));
        assert!(
            after.is_checklist,
            "legacy payloads without is_checklist must not demote locally-known checklists"
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

        let base_md = crate::services::checklist_md::serialize(&[
            crate::services::checklist_md::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist_md::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let local_md = crate::services::checklist_md::set_checked(&base_md, "a1", true);

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
        local_quest.description = Some(local_md.clone());
        local_quest.checklist_base = Some(base_md.clone());
        insert_local_quest_row(&mut conn, &local_quest);

        let remote_md = crate::services::checklist_md::set_checked(&base_md, "a2", true);
        let mut remote_quest = test_quest("q1", "Stale remote title", "2026-03-01T00:00:00Z");
        remote_quest.is_checklist = true;
        remote_quest.description = Some(remote_md);
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
        let items = crate::services::checklist_md::parse(after.description.as_deref().unwrap());
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

    #[test]
    fn apply_sync_event_emits_convergence_event_when_checklist_base_advances_only() {
        let db_path = temp_db_path("checklist-base-ack");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();
        crate::services::settings::upsert_setting(&mut conn, "device.id", "dev-local")
            .expect("seed local device id");

        let base_md = crate::services::checklist_md::serialize(&[
            crate::services::checklist_md::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            crate::services::checklist_md::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let adopted_md = crate::services::checklist_md::set_checked(&base_md, "a1", true);

        let mut local_quest = test_quest("q1", "Go to office", "2026-03-02T00:00:00Z");
        local_quest.is_checklist = true;
        local_quest.description = Some(adopted_md.clone());
        local_quest.checklist_base = Some(base_md);
        insert_local_quest_row(&mut conn, &local_quest);

        let mut incoming_quest = test_quest("q1", "Go to office", "2026-03-02T00:00:00Z");
        incoming_quest.is_checklist = true;
        incoming_quest.description = Some(adopted_md.clone());
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
        assert_eq!(after.description.as_deref(), Some(adopted_md.as_str()));
        assert_eq!(after.checklist_base.as_deref(), Some(adopted_md.as_str()));

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
        assert_eq!(payload.description.as_deref(), Some(adopted_md.as_str()));

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

    #[test]
    fn space_sync_status_reports_last_synced_per_space() {
        let db_path = temp_db_path("status-last-synced-by-space");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-a"),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(&mut conn)
            .unwrap();

        diesel::insert_into(pair_space_mappings::table)
            .values(vec![
                (
                    pair_space_mappings::peer_device_id.eq("peer-a"),
                    pair_space_mappings::space_id.eq("1"),
                    pair_space_mappings::enabled_at.eq("2026-04-07T00:00:00Z"),
                    pair_space_mappings::last_synced_at.eq(Some("2026-04-07T10:01:00Z")),
                ),
                (
                    pair_space_mappings::peer_device_id.eq("peer-a"),
                    pair_space_mappings::space_id.eq("2"),
                    pair_space_mappings::enabled_at.eq("2026-04-07T00:00:00Z"),
                    pair_space_mappings::last_synced_at.eq(Some("2026-04-07T10:06:00Z")),
                ),
            ])
            .execute(&mut conn)
            .unwrap();

        let status = space_sync_status_impl(&mut conn, Some("peer-a".to_string())).unwrap();

        assert_eq!(
            status.last_synced_at,
            Some("2026-04-07T10:06:00Z".to_string())
        );
        assert_eq!(
            status.last_synced_at_by_space.get("1"),
            Some(&Some("2026-04-07T10:01:00Z".to_string()))
        );
        assert_eq!(
            status.last_synced_at_by_space.get("2"),
            Some(&Some("2026-04-07T10:06:00Z".to_string()))
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn space_sync_tick_defers_incoming_events_until_mapping_is_approved() {
        let db_path = temp_db_path("tick-gated-incoming");
        let app_dir = temp_app_dir("tick-gated-incoming");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-a"),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(&mut conn)
            .unwrap();

        let device_connection = DeviceConnectionState::from_app_data_dir(&app_dir);
        let quest = test_quest("q-gated", "Blocked Until Approval", "2026-04-11T20:00:00Z");
        let event = test_envelope(
            "evt-gated",
            "peer-a",
            "quest",
            "q-gated",
            "upsert",
            Some(quest_payload(&quest)),
            "2026-04-11T20:00:00Z",
        );

        device_connection.restore_incoming_sync_events(vec![event]);

        let first_tick = space_sync_tick_impl(&mut conn, &device_connection).unwrap();
        assert_eq!(first_tick.applied_events, 0);

        {
            let count: i64 = quests::table
                .filter(quests::id.eq("q-gated"))
                .count()
                .get_result(&mut conn)
                .unwrap();
            assert_eq!(count, 0);

            diesel::insert_into(pair_space_mappings::table)
                .values((
                    pair_space_mappings::peer_device_id.eq("peer-a"),
                    pair_space_mappings::space_id.eq("1"),
                    pair_space_mappings::enabled_at.eq("2026-04-11T20:01:00Z"),
                    pair_space_mappings::last_synced_at.eq(Option::<String>::None),
                ))
                .execute(&mut conn)
                .unwrap();
        }

        let second_tick = space_sync_tick_impl(&mut conn, &device_connection).unwrap();
        assert_eq!(second_tick.applied_events, 1);

        {
            let count: i64 = quests::table
                .filter(quests::id.eq("q-gated"))
                .count()
                .get_result(&mut conn)
                .unwrap();
            assert_eq!(count, 1);
        }

        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_dir_all(app_dir);
    }

    #[test]
    fn space_sync_tick_does_not_replay_mapping_snapshot_when_session_appears() {
        let db_path = temp_db_path("tick-does-not-replay-mapping-snapshot");
        let app_dir = temp_app_dir("tick-does-not-replay-mapping-snapshot");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-a"),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(&mut conn)
            .unwrap();

        diesel::insert_into(pair_space_mappings::table)
            .values((
                pair_space_mappings::peer_device_id.eq("peer-a"),
                pair_space_mappings::space_id.eq("1"),
                pair_space_mappings::enabled_at.eq("2026-04-07T00:00:00Z"),
                pair_space_mappings::last_synced_at.eq(Option::<String>::None),
            ))
            .execute(&mut conn)
            .unwrap();

        let device_connection = DeviceConnectionState::from_app_data_dir(&app_dir);
        let (tx, mut rx) = mpsc::channel(4);
        device_connection.register_session("peer-a".to_string(), tx);

        let tick = space_sync_tick_impl(&mut conn, &device_connection).unwrap();
        assert_eq!(tick.sent_events, 0);

        while let Ok(sent) = rx.try_recv() {
            if matches!(sent, WsMessage::SpaceMappingUpdate { .. }) {
                panic!("mapping snapshot should not be sent");
            }
        }

        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_dir_all(app_dir);
    }
}
