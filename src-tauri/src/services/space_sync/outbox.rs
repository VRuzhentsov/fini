use diesel::prelude::*;
use diesel::SqliteConnection;
use uuid::Uuid;

use crate::models::CreateSyncOutboxEntry;
use crate::schema::sync_outbox;
use crate::services::db::utc_now;

use super::types::SyncEventEnvelope;

fn normalize_legacy_sync_payload(
    entity_type: &str,
    payload: Option<String>,
) -> Result<Option<String>, String> {
    if !matches!(entity_type, "quest" | "quest_series") {
        return Ok(payload);
    }
    let Some(payload) = payload else {
        return Ok(None);
    };
    let mut value: serde_json::Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "sync metadata payload must be a JSON object".to_string())?;
    if let Some(serde_json::Value::String(energy)) = object.get("energy") {
        let legacy = match energy.as_str() {
            "small" => "low",
            "large" => "high",
            _ => energy,
        };
        object.insert(
            "energy".to_string(),
            serde_json::Value::String(legacy.to_string()),
        );
    }
    if let Some(serde_json::Value::String(priority)) = object.get("priority") {
        let legacy = match priority.as_str() {
            "low" => 2,
            "high" => 4,
            _ => 3,
        };
        object.insert("priority".to_string(), serde_json::Value::from(legacy));
    }
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|e| e.to_string())
}

pub fn emit_sync_event_at(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    entity_type: &str,
    entity_id: &str,
    space_id: &str,
    op_type: &str,
    payload: Option<String>,
    updated_at: String,
) -> Result<(), String> {
    let payload = normalize_legacy_sync_payload(entity_type, payload)?;
    let entry = CreateSyncOutboxEntry {
        event_id: Uuid::new_v4().to_string(),
        correlation_id: Uuid::new_v4().to_string(),
        origin_device_id: origin_device_id.to_string(),
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        space_id: space_id.to_string(),
        op_type: op_type.to_string(),
        payload,
        updated_at,
    };

    diesel::insert_into(sync_outbox::table)
        .values(&entry)
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn emit_sync_event(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    entity_type: &str,
    entity_id: &str,
    space_id: &str,
    op_type: &str,
    payload: Option<String>,
) -> Result<(), String> {
    emit_sync_event_at(
        conn,
        origin_device_id,
        entity_type,
        entity_id,
        space_id,
        op_type,
        payload,
        utc_now(),
    )
}

pub fn load_unacked_events_for_peer(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    mapped_space_ids: &[String],
) -> Result<Vec<SyncEventEnvelope>, String> {
    use crate::schema::sync_acks;

    if mapped_space_ids.is_empty() {
        return Ok(vec![]);
    }

    let acked_event_ids: Vec<String> = sync_acks::table
        .filter(sync_acks::peer_device_id.eq(peer_device_id))
        .select(sync_acks::event_id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let mut query = sync_outbox::table
        .filter(sync_outbox::space_id.eq_any(mapped_space_ids))
        .order(sync_outbox::created_at.asc())
        .into_boxed();

    if !acked_event_ids.is_empty() {
        query = query.filter(sync_outbox::event_id.ne_all(&acked_event_ids));
    }

    let rows: Vec<crate::models::SyncOutboxEntry> = query
        .select(crate::models::SyncOutboxEntry::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|r| SyncEventEnvelope {
            event_id: r.event_id,
            correlation_id: r.correlation_id,
            origin_device_id: r.origin_device_id,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            space_id: r.space_id,
            op_type: r.op_type,
            payload: r.payload,
            updated_at: r.updated_at,
            created_at: r.created_at,
        })
        .collect())
}

pub fn load_latest_event_for_entity(
    conn: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> Result<Option<SyncEventEnvelope>, String> {
    let row: Option<crate::models::SyncOutboxEntry> = sync_outbox::table
        .filter(sync_outbox::entity_type.eq(entity_type))
        .filter(sync_outbox::entity_id.eq(entity_id))
        .order(sync_outbox::updated_at.desc())
        .select(crate::models::SyncOutboxEntry::as_select())
        .first(conn)
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(row.map(|r| SyncEventEnvelope {
        event_id: r.event_id,
        correlation_id: r.correlation_id,
        origin_device_id: r.origin_device_id,
        entity_type: r.entity_type,
        entity_id: r.entity_id,
        space_id: r.space_id,
        op_type: r.op_type,
        payload: r.payload,
        updated_at: r.updated_at,
        created_at: r.created_at,
    }))
}

pub fn load_events_for_space(
    conn: &mut SqliteConnection,
    space_id: &str,
) -> Result<Vec<SyncEventEnvelope>, String> {
    let rows: Vec<crate::models::SyncOutboxEntry> = sync_outbox::table
        .filter(sync_outbox::space_id.eq(space_id))
        .order(sync_outbox::created_at.asc())
        .select(crate::models::SyncOutboxEntry::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|r| SyncEventEnvelope {
            event_id: r.event_id,
            correlation_id: r.correlation_id,
            origin_device_id: r.origin_device_id,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            space_id: r.space_id,
            op_type: r.op_type,
            payload: r.payload,
            updated_at: r.updated_at,
            created_at: r.created_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::open_db_at_path;
    use std::path::PathBuf;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fini-test-outbox-{label}-{}.db", Uuid::new_v4()))
    }

    #[test]
    fn quest_metadata_sync_payloads_stay_compatible_with_pre_v21_peers() {
        let db_path = temp_db_path("legacy-metadata-wire");
        let mut conn = open_db_at_path(&db_path);
        emit_sync_event(
            &mut conn,
            "device-a",
            "quest",
            "quest-1",
            "1",
            "upsert",
            Some(r#"{"energy":"large","priority":"high"}"#.to_string()),
        )
        .expect("emit quest sync event");
        let events = load_unacked_events_for_peer(&mut conn, "device-b", &["1".to_string()])
            .expect("load unacked");
        let payload: serde_json::Value =
            serde_json::from_str(events[0].payload.as_deref().expect("quest payload"))
                .expect("decode normalized payload");
        assert_eq!(payload["energy"], "high");
        assert_eq!(payload["priority"], 4);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn emit_and_load_sync_events() {
        let db_path = temp_db_path("emit-load");
        let mut conn = open_db_at_path(&db_path);

        emit_sync_event(
            &mut conn,
            "device-a",
            "quest",
            "quest-1",
            "1",
            "upsert",
            Some(r#"{"title":"Test"}"#.to_string()),
        )
        .expect("emit sync event");

        let events = load_unacked_events_for_peer(&mut conn, "device-b", &["1".to_string()])
            .expect("load unacked");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_type, "quest");
        assert_eq!(events[0].entity_id, "quest-1");
        assert_eq!(events[0].op_type, "upsert");

        let _ = std::fs::remove_file(db_path);
    }
}
