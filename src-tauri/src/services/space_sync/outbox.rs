use diesel::prelude::*;
use diesel::SqliteConnection;
use uuid::Uuid;

use crate::models::CreateSyncOutboxEntry;
use crate::schema::sync_outbox;
use crate::services::db::utc_now;

use super::types::SyncEventEnvelope;

pub fn emit_sync_event(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    entity_type: &str,
    entity_id: &str,
    space_id: &str,
    op_type: &str,
    payload: Option<String>,
) -> Result<(), String> {
    let now = utc_now();
    let entry = CreateSyncOutboxEntry {
        event_id: Uuid::new_v4().to_string(),
        correlation_id: Uuid::new_v4().to_string(),
        origin_device_id: origin_device_id.to_string(),
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        space_id: space_id.to_string(),
        op_type: op_type.to_string(),
        payload,
        updated_at: now,
    };

    diesel::insert_into(sync_outbox::table)
        .values(&entry)
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
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
