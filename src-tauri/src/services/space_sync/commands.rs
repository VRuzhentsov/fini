use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use tauri::State;

use super::merge::incoming_wins;
use super::outbox::{emit_sync_event, load_latest_event_for_entity, load_unacked_events_for_peer};
use super::replay::{is_event_seen, mark_event_seen, record_ack};
use super::types::SyncEventEnvelope;
use crate::models::{
    CreatePairSpaceMappingInput, FocusHistoryEntry, Quest, QuestSeries, Reminder, Space,
};
use crate::schema::{
    focus_history, pair_space_mappings, paired_devices, quest_series, quests, reminders, spaces,
    sync_acks, sync_outbox, sync_seen, tombstones,
};
use crate::services::db::{utc_now, DbState};
use crate::services::device_connection::{
    DeviceConnectionState, DISCOVERY_PORT, DISCOVERY_PROTOCOL, SPACE_MAPPING_UPDATE_KIND,
    SYNC_ACK_KIND, SYNC_EVENT_KIND,
};

const MAX_EVENTS_PER_PEER_PER_TICK: usize = 64;

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncStatus {
    pub peer_device_id: Option<String>,
    pub mapped_space_ids: Vec<String>,
    pub last_synced_at: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSpaceDescriptor {
    pub space_id: String,
    pub name: String,
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
        .order(pair_space_mappings::space_id.asc())
        .select(pair_space_mappings::space_id)
        .load(conn)
        .map_err(|e| e.to_string())
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
    origin_device_id: &str,
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
    let existing_set: BTreeSet<String> = existing_ids.into_iter().collect();
    let desired_set: BTreeSet<String> = desired_ids.iter().cloned().collect();

    let to_add: Vec<String> = desired_set.difference(&existing_set).cloned().collect();
    let to_remove: Vec<String> = existing_set.difference(&desired_set).cloned().collect();

    if !to_remove.is_empty() {
        diesel::delete(
            pair_space_mappings::table
                .filter(pair_space_mappings::peer_device_id.eq(peer_device_id))
                .filter(pair_space_mappings::space_id.eq_any(&to_remove)),
        )
        .execute(conn)
        .map_err(|e| e.to_string())?;
    }

    if !to_add.is_empty() {
        let now = utc_now();
        let rows: Vec<CreatePairSpaceMappingInput> = to_add
            .iter()
            .map(|space_id| CreatePairSpaceMappingInput {
                peer_device_id: peer_device_id.to_string(),
                space_id: space_id.clone(),
                enabled_at: now.clone(),
            })
            .collect();

        diesel::insert_or_ignore_into(pair_space_mappings::table)
            .values(&rows)
            .execute(conn)
            .map_err(|e| e.to_string())?;

        for space_id in &to_add {
            bootstrap_space_into_outbox(conn, origin_device_id, space_id)?;
        }
    }

    list_mappings_for_peer(conn, peer_device_id)
}

fn send_mapping_update_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    mapped_space_ids: &[String],
    custom_spaces: &[CustomSpaceDescriptor],
) -> Result<(), String> {
    let Some(peer_addr) = device_connection.resolve_peer_addr(peer_device_id) else {
        return Ok(());
    };

    let target_ip: IpAddr = peer_addr
        .parse()
        .map_err(|err| format!("invalid peer addr '{peer_addr}': {err}"))?;

    let payload = serde_json::json!({
        "protocol": DISCOVERY_PROTOCOL,
        "kind": SPACE_MAPPING_UPDATE_KIND,
        "from_device_id": device_connection.identity.device_id,
        "to_device_id": peer_device_id,
        "mapped_space_ids": mapped_space_ids,
        "custom_spaces": custom_spaces,
        "sent_at": utc_now(),
    });

    let bytes =
        serde_json::to_vec(&payload).map_err(|err| format!("serialize mapping update: {err}"))?;
    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("bind mapping update socket failed: {err}"))?;
    let target = SocketAddr::new(target_ip, DISCOVERY_PORT);

    let mut sent_count = 0;
    let mut last_send_error: Option<String> = None;
    for _ in 0..3 {
        match socket.send_to(&bytes, target) {
            Ok(_) => sent_count += 1,
            Err(err) => last_send_error = Some(err.to_string()),
        }
    }

    if sent_count == 0 {
        let message = last_send_error.unwrap_or_else(|| "unknown send error".to_string());
        return Err(format!("send mapping update failed: {message}"));
    }

    Ok(())
}

fn send_sync_event_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    event: &SyncEventEnvelope,
) -> Result<(), String> {
    let Some(peer_addr) = device_connection.resolve_peer_addr(peer_device_id) else {
        return Ok(());
    };

    let target_ip: IpAddr = peer_addr
        .parse()
        .map_err(|err| format!("invalid peer addr '{peer_addr}': {err}"))?;

    let payload = serde_json::json!({
        "protocol": DISCOVERY_PROTOCOL,
        "kind": SYNC_EVENT_KIND,
        "to_device_id": peer_device_id,
        "event": event,
    });

    let bytes =
        serde_json::to_vec(&payload).map_err(|err| format!("serialize sync event: {err}"))?;
    let socket =
        UdpSocket::bind(("0.0.0.0", 0)).map_err(|err| format!("bind sync socket failed: {err}"))?;
    let target = SocketAddr::new(target_ip, DISCOVERY_PORT);
    socket
        .send_to(&bytes, target)
        .map_err(|err| format!("send sync event failed: {err}"))?;
    Ok(())
}

fn send_sync_ack_to_peer(
    device_connection: &DeviceConnectionState,
    peer_device_id: &str,
    event_id: &str,
) -> Result<(), String> {
    let Some(peer_addr) = device_connection.resolve_peer_addr(peer_device_id) else {
        return Ok(());
    };

    let target_ip: IpAddr = peer_addr
        .parse()
        .map_err(|err| format!("invalid peer addr '{peer_addr}': {err}"))?;

    let payload = serde_json::json!({
        "protocol": DISCOVERY_PROTOCOL,
        "kind": SYNC_ACK_KIND,
        "from_device_id": device_connection.identity.device_id,
        "to_device_id": peer_device_id,
        "event_id": event_id,
        "acked_at": utc_now(),
    });

    let bytes = serde_json::to_vec(&payload).map_err(|err| format!("serialize sync ack: {err}"))?;
    let socket =
        UdpSocket::bind(("0.0.0.0", 0)).map_err(|err| format!("bind ack socket failed: {err}"))?;
    let target = SocketAddr::new(target_ip, DISCOVERY_PORT);
    socket
        .send_to(&bytes, target)
        .map_err(|err| format!("send sync ack failed: {err}"))?;
    Ok(())
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
            quests::set_focus_at.eq(&quest.set_focus_at),
            quests::reminder_triggered_at.eq(&quest.reminder_triggered_at),
            quests::order_rank.eq(quest.order_rank),
            quests::created_at.eq(&quest.created_at),
            quests::updated_at.eq(&quest.updated_at),
            quests::series_id.eq(&quest.series_id),
            quests::period_key.eq(&quest.period_key),
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
            quests::set_focus_at.eq(&quest.set_focus_at),
            quests::reminder_triggered_at.eq(&quest.reminder_triggered_at),
            quests::order_rank.eq(quest.order_rank),
            quests::created_at.eq(&quest.created_at),
            quests::updated_at.eq(&quest.updated_at),
            quests::series_id.eq(&quest.series_id),
            quests::period_key.eq(&quest.period_key),
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

fn upsert_reminder(conn: &mut SqliteConnection, reminder: &Reminder) -> Result<(), String> {
    diesel::insert_into(reminders::table)
        .values((
            reminders::id.eq(&reminder.id),
            reminders::quest_id.eq(&reminder.quest_id),
            reminders::kind.eq(&reminder.kind),
            reminders::mm_offset.eq(reminder.mm_offset),
            reminders::due_at_utc.eq(&reminder.due_at_utc),
            reminders::created_at.eq(&reminder.created_at),
        ))
        .on_conflict(reminders::id)
        .do_update()
        .set((
            reminders::quest_id.eq(&reminder.quest_id),
            reminders::kind.eq(&reminder.kind),
            reminders::mm_offset.eq(reminder.mm_offset),
            reminders::due_at_utc.eq(&reminder.due_at_utc),
            reminders::created_at.eq(&reminder.created_at),
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
            focus_history::device_id.eq(&focus.device_id),
            focus_history::quest_id.eq(&focus.quest_id),
            focus_history::space_id.eq(&focus.space_id),
            focus_history::trigger.eq(&focus.trigger),
            focus_history::created_at.eq(&focus.created_at),
        ))
        .on_conflict(focus_history::id)
        .do_update()
        .set((
            focus_history::device_id.eq(&focus.device_id),
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
        "reminder" => {
            diesel::delete(reminders::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        "focus_history" => {
            diesel::delete(focus_history::table.find(&event.entity_id))
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
            if let Some(local_event) =
                load_latest_event_for_entity(conn, &event.entity_type, &event.entity_id)?
            {
                if !incoming_wins(event, &local_event) {
                    mark_event_seen(conn, &event.event_id)?;
                    return Ok(false);
                }
            }

            let payload = event
                .payload
                .as_ref()
                .ok_or_else(|| "missing payload for upsert event".to_string())?;

            match event.entity_type.as_str() {
                "space" => {
                    let space: Space = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    upsert_space(conn, &space)?;
                }
                "quest" => {
                    let quest: Quest = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    upsert_quest(conn, &quest)?;
                }
                "quest_series" => {
                    let series: QuestSeries =
                        serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    upsert_quest_series(conn, &series)?;
                }
                "reminder" => {
                    let reminder: Reminder =
                        serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    upsert_reminder(conn, &reminder)?;
                }
                "focus_history" => {
                    let focus: FocusHistoryEntry =
                        serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    upsert_focus_history(conn, &focus)?;
                }
                _ => {}
            }
        }
        _ => {}
    }

    mark_event_seen(conn, &event.event_id)?;
    Ok(true)
}

fn enqueue_upsert<T: Serialize>(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    entity_type: &str,
    entity_id: &str,
    space_id: &str,
    value: &T,
) -> Result<(), String> {
    let payload = serde_json::to_string(value).map_err(|e| e.to_string())?;
    emit_sync_event(
        conn,
        origin_device_id,
        entity_type,
        entity_id,
        space_id,
        "upsert",
        Some(payload),
    )
}

fn bootstrap_space_into_outbox(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    space_id: &str,
) -> Result<(), String> {
    let space: Space = spaces::table
        .find(space_id)
        .select(Space::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;
    enqueue_upsert(
        conn,
        origin_device_id,
        "space",
        &space.id,
        &space.id,
        &space,
    )?;

    let quest_rows: Vec<Quest> = quests::table
        .filter(quests::space_id.eq(space_id))
        .select(Quest::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;
    for quest in quest_rows {
        enqueue_upsert(
            conn,
            origin_device_id,
            "quest",
            &quest.id,
            &quest.space_id,
            &quest,
        )?;
    }

    let series_rows: Vec<QuestSeries> = quest_series::table
        .filter(quest_series::space_id.eq(space_id))
        .select(QuestSeries::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;
    for series in series_rows {
        enqueue_upsert(
            conn,
            origin_device_id,
            "quest_series",
            &series.id,
            &series.space_id,
            &series,
        )?;
    }

    let reminder_rows: Vec<Reminder> = reminders::table
        .inner_join(quests::table.on(reminders::quest_id.eq(quests::id)))
        .filter(quests::space_id.eq(space_id))
        .select(Reminder::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;
    for reminder in reminder_rows {
        enqueue_upsert(
            conn,
            origin_device_id,
            "reminder",
            &reminder.id,
            space_id,
            &reminder,
        )?;
    }

    let focus_rows: Vec<FocusHistoryEntry> = focus_history::table
        .filter(focus_history::space_id.eq(space_id))
        .select(FocusHistoryEntry::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;
    for focus in focus_rows {
        enqueue_upsert(
            conn,
            origin_device_id,
            "focus_history",
            &focus.id,
            &focus.space_id,
            &focus,
        )?;
    }

    Ok(())
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
    db: &DbState,
    peer_device_id: String,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().unwrap();
    list_mappings_for_peer(&mut conn, &peer_device_id)
}

#[tauri::command]
pub fn space_sync_list_mappings(
    db: State<DbState>,
    peer_device_id: String,
) -> Result<Vec<String>, String> {
    space_sync_list_mappings_impl(&db, peer_device_id)
}

pub fn space_sync_update_mappings_impl(
    db: &DbState,
    device_connection: &DeviceConnectionState,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().unwrap();
    let origin_device_id = device_connection.identity.device_id.clone();

    let mapped = apply_mappings_in_db(
        &mut conn,
        &origin_device_id,
        &peer_device_id,
        mapped_space_ids,
    )?;

    let custom_spaces = load_custom_space_descriptors(&mut conn, &mapped)?;

    if let Err(err) =
        send_mapping_update_to_peer(&device_connection, &peer_device_id, &mapped, &custom_spaces)
    {
        eprintln!("[space-sync] failed to notify peer mapping update: {err}");
    }

    Ok(mapped)
}

#[tauri::command]
pub fn space_sync_update_mappings(
    db: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    space_sync_update_mappings_impl(&db, &device_connection, peer_device_id, mapped_space_ids)
}

pub fn space_sync_apply_remote_mappings_impl(
    db: &DbState,
    device_connection: &DeviceConnectionState,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<SpaceMappingApplyResult, String> {
    let mut conn = db.0.lock().unwrap();
    let origin_device_id = device_connection.identity.device_id.clone();

    let (resolved_ids, unresolved_custom_spaces) =
        partition_remote_mapped_spaces(&mut conn, mapped_space_ids, custom_spaces)?;

    let mapped_space_ids =
        apply_mappings_in_db(&mut conn, &origin_device_id, &peer_device_id, resolved_ids)?;

    Ok(SpaceMappingApplyResult {
        mapped_space_ids,
        unresolved_custom_spaces,
    })
}

#[tauri::command]
pub fn space_sync_apply_remote_mappings(
    db: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<SpaceMappingApplyResult, String> {
    space_sync_apply_remote_mappings_impl(
        &db,
        &device_connection,
        peer_device_id,
        mapped_space_ids,
        custom_spaces,
    )
}

pub fn space_sync_resolve_custom_space_mapping_impl(
    db: &DbState,
    device_connection: &DeviceConnectionState,
    peer_device_id: String,
    remote_space_id: String,
    remote_space_name: Option<String>,
    resolution_mode: SpaceResolutionMode,
    existing_space_id: Option<String>,
) -> Result<SpaceMappingApplyResult, String> {
    let mut conn = db.0.lock().unwrap();
    let origin_device_id = device_connection.identity.device_id.clone();

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
            create_space_with_id(&mut conn, &remote_space_id, &name)?;
        }
        SpaceResolutionMode::UseExisting => {
            let local_space_id = existing_space_id
                .as_deref()
                .ok_or_else(|| "existing_space_id is required for use_existing".to_string())?;
            merge_local_space_into_remote_id(
                &mut conn,
                local_space_id,
                &remote_space_id,
                remote_space_name.as_deref(),
            )?;
        }
    }

    let mut mapped = list_mappings_for_peer(&mut conn, &peer_device_id)?;
    if !mapped.contains(&remote_space_id) {
        mapped.push(remote_space_id.clone());
    }

    let mapped_space_ids =
        apply_mappings_in_db(&mut conn, &origin_device_id, &peer_device_id, mapped)?;

    let custom_spaces = load_custom_space_descriptors(&mut conn, &mapped_space_ids)?;
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

#[tauri::command]
pub fn space_sync_resolve_custom_space_mapping(
    db: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    remote_space_id: String,
    remote_space_name: Option<String>,
    resolution_mode: SpaceResolutionMode,
    existing_space_id: Option<String>,
) -> Result<SpaceMappingApplyResult, String> {
    space_sync_resolve_custom_space_mapping_impl(
        &db,
        &device_connection,
        peer_device_id,
        remote_space_id,
        remote_space_name,
        resolution_mode,
        existing_space_id,
    )
}

pub fn space_sync_tick_impl(
    db: &DbState,
    device_connection: &DeviceConnectionState,
) -> Result<SpaceSyncTickResult, String> {
    let mut conn = db.0.lock().unwrap();
    let ticked_at = utc_now();
    let mut transferred_by_peer: std::collections::HashMap<String, (usize, usize, usize)> =
        std::collections::HashMap::new();

    let peer_ids: Vec<String> = paired_devices::table
        .select(paired_devices::peer_device_id)
        .load(&mut *conn)
        .map_err(|e| e.to_string())?;

    let mut sent_events_total = 0usize;

    for peer_device_id in peer_ids {
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
                .entry(peer_device_id)
                .or_insert((0, 0, 0));
            entry.0 += sent_events;
        }
    }

    let incoming_events = device_connection.take_incoming_sync_events();
    let mut applied_events = 0usize;
    for event in incoming_events {
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

    let incoming_acks = device_connection.take_incoming_sync_acks();
    let mut received_acks = 0usize;
    for ack in incoming_acks {
        if let Ok(inserted) = record_ack(&mut conn, &ack.from_device_id, &ack.event_id) {
            if !inserted {
                continue;
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

#[tauri::command]
pub fn space_sync_tick(
    db: State<DbState>,
    device_connection: State<DeviceConnectionState>,
) -> Result<SpaceSyncTickResult, String> {
    space_sync_tick_impl(&db, &device_connection)
}

pub fn space_sync_status_impl(
    db: &DbState,
    peer_device_id: Option<String>,
) -> Result<SpaceSyncStatus, String> {
    let mut conn = db.0.lock().unwrap();

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

    let (mapped_space_ids, last_synced_at, pending_event_count, acked_event_count) =
        if let Some(peer_id) = peer_device_id.as_deref() {
            let mapped = list_mappings_for_peer(&mut conn, peer_id)?;
            let last_synced = sync_acks::table
                .filter(sync_acks::peer_device_id.eq(peer_id))
                .select(sync_acks::acked_at)
                .order(sync_acks::acked_at.desc())
                .first::<String>(&mut *conn)
                .optional()
                .map_err(|e| e.to_string())?;
            let pending = load_unacked_events_for_peer(&mut conn, peer_id, &mapped)?.len();
            let acked: i64 = sync_acks::table
                .filter(sync_acks::peer_device_id.eq(peer_id))
                .count()
                .get_result(&mut *conn)
                .map_err(|e| e.to_string())?;
            (mapped, last_synced, pending, acked)
        } else {
            (Vec::new(), None, 0, 0)
        };

    Ok(SpaceSyncStatus {
        peer_device_id,
        mapped_space_ids,
        last_synced_at,
        pending_event_count,
        outbox_event_count,
        acked_event_count,
        seen_event_count,
        tombstone_count,
    })
}

#[tauri::command]
pub fn space_sync_status(
    db: State<DbState>,
    peer_device_id: Option<String>,
) -> Result<SpaceSyncStatus, String> {
    space_sync_status_impl(&db, peer_device_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CreateSyncOutboxEntry, SyncOutboxEntry};
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
            energy: "medium".to_string(),
            priority: 1,
            pinned: false,
            due: None,
            due_time: None,
            repeat_rule: None,
            completed_at: None,
            set_focus_at: None,
            reminder_triggered_at: None,
            order_rank: 1000.0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
            series_id: None,
            period_key: None,
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

    fn insert_outbox_entry(
        conn: &mut SqliteConnection,
        origin_device_id: &str,
        entity_type: &str,
        entity_id: &str,
        updated_at: &str,
        payload: Option<String>,
    ) {
        let entry = CreateSyncOutboxEntry {
            event_id: Uuid::new_v4().to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            origin_device_id: origin_device_id.to_string(),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            space_id: "1".to_string(),
            op_type: "upsert".to_string(),
            payload,
            updated_at: updated_at.to_string(),
        };
        diesel::insert_into(sync_outbox::table)
            .values(&entry)
            .execute(conn)
            .unwrap();
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
    fn bootstrap_space_emits_all_entity_types() {
        let db_path = temp_db_path("bootstrap");
        let mut conn = open_db_at_path(&db_path);
        ensure_spaces_exist(&mut conn, &["1".to_string()]).unwrap();

        let quest = test_quest("q-boot", "Boot", "2026-01-01T00:00:00Z");
        upsert_quest(&mut conn, &quest).unwrap();

        bootstrap_space_into_outbox(&mut conn, "dev-local", "1").unwrap();

        let events: Vec<SyncOutboxEntry> = sync_outbox::table
            .select(SyncOutboxEntry::as_select())
            .load(&mut conn)
            .unwrap();

        let types: Vec<&str> = events.iter().map(|e| e.entity_type.as_str()).collect();
        assert!(types.contains(&"space"));
        assert!(types.contains(&"quest"));

        let _ = std::fs::remove_file(db_path);
    }
}
