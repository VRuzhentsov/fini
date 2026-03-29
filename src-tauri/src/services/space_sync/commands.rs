use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::Serialize;
use std::collections::BTreeSet;
use tauri::State;

use super::outbox::{emit_sync_event, load_unacked_events_for_peer};
use crate::models::{
    CreatePairSpaceMappingInput, FocusHistoryEntry, Quest, QuestSeries, Reminder, Space,
};
use crate::schema::{
    focus_history, pair_space_mappings, paired_devices, quest_series, quests, reminders, spaces,
    sync_acks, sync_outbox, sync_seen, tombstones,
};
use crate::services::db::{utc_now, DbState};
use crate::services::device_connection::DeviceConnectionState;

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncStatus {
    pub peer_device_id: Option<String>,
    pub mapped_space_ids: Vec<String>,
    pub pending_event_count: usize,
    pub outbox_event_count: i64,
    pub acked_event_count: i64,
    pub seen_event_count: i64,
    pub tombstone_count: i64,
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

#[tauri::command]
pub fn space_sync_list_mappings(
    db: State<DbState>,
    peer_device_id: String,
) -> Result<Vec<String>, String> {
    let mut conn = db.inner().0.lock().unwrap();
    list_mappings_for_peer(&mut conn, &peer_device_id)
}

#[tauri::command]
pub fn space_sync_update_mappings(
    db: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    peer_device_id: String,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut conn = db.inner().0.lock().unwrap();

    let peer_exists = paired_devices::table
        .find(&peer_device_id)
        .count()
        .get_result::<i64>(&mut *conn)
        .map_err(|e| e.to_string())?
        > 0;
    if !peer_exists {
        return Err("paired device not found".to_string());
    }

    let desired_ids = normalized_space_ids(mapped_space_ids);
    if !desired_ids.is_empty() {
        let existing_space_ids: Vec<String> = spaces::table
            .filter(spaces::id.eq_any(&desired_ids))
            .select(spaces::id)
            .load(&mut *conn)
            .map_err(|e| e.to_string())?;

        if existing_space_ids.len() != desired_ids.len() {
            let known: BTreeSet<String> = existing_space_ids.into_iter().collect();
            let missing: Vec<String> = desired_ids
                .iter()
                .filter(|id| !known.contains((*id).as_str()))
                .cloned()
                .collect();
            return Err(format!("unknown space ids: {}", missing.join(", ")));
        }
    }

    let existing_ids = list_mappings_for_peer(&mut conn, &peer_device_id)?;
    let existing_set: BTreeSet<String> = existing_ids.into_iter().collect();
    let desired_set: BTreeSet<String> = desired_ids.iter().cloned().collect();

    let to_add: Vec<String> = desired_set.difference(&existing_set).cloned().collect();
    let to_remove: Vec<String> = existing_set.difference(&desired_set).cloned().collect();

    if !to_remove.is_empty() {
        diesel::delete(
            pair_space_mappings::table
                .filter(pair_space_mappings::peer_device_id.eq(&peer_device_id))
                .filter(pair_space_mappings::space_id.eq_any(&to_remove)),
        )
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    }

    if !to_add.is_empty() {
        let now = utc_now();
        let rows: Vec<CreatePairSpaceMappingInput> = to_add
            .iter()
            .map(|space_id| CreatePairSpaceMappingInput {
                peer_device_id: peer_device_id.clone(),
                space_id: space_id.clone(),
                enabled_at: now.clone(),
            })
            .collect();

        diesel::insert_or_ignore_into(pair_space_mappings::table)
            .values(&rows)
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;

        let origin_device_id = device_connection.identity.device_id.clone();
        for space_id in &to_add {
            bootstrap_space_into_outbox(&mut conn, &origin_device_id, space_id)?;
        }
    }

    list_mappings_for_peer(&mut conn, &peer_device_id)
}

#[tauri::command]
pub fn space_sync_status(
    db: State<DbState>,
    peer_device_id: Option<String>,
) -> Result<SpaceSyncStatus, String> {
    let mut conn = db.inner().0.lock().unwrap();

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

    let (mapped_space_ids, pending_event_count, acked_event_count) =
        if let Some(peer_id) = peer_device_id.as_deref() {
            let mapped = list_mappings_for_peer(&mut conn, peer_id)?;
            let pending = load_unacked_events_for_peer(&mut conn, peer_id, &mapped)?.len();
            let acked: i64 = sync_acks::table
                .filter(sync_acks::peer_device_id.eq(peer_id))
                .count()
                .get_result(&mut *conn)
                .map_err(|e| e.to_string())?;
            (mapped, pending, acked)
        } else {
            (Vec::new(), 0, 0)
        };

    Ok(SpaceSyncStatus {
        peer_device_id,
        mapped_space_ids,
        pending_event_count,
        outbox_event_count,
        acked_event_count,
        seen_event_count,
        tombstone_count,
    })
}
