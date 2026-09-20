use diesel::prelude::*;
use diesel::SqliteConnection;

use crate::models::{CreateSyncAck, CreateSyncSeen};
use crate::schema::{sync_acks, sync_seen};
use crate::services::db::utc_now;

pub fn record_ack(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    event_id: &str,
) -> Result<bool, String> {
    let ack = CreateSyncAck {
        peer_device_id: peer_device_id.to_string(),
        event_id: event_id.to_string(),
        acked_at: utc_now(),
    };

    let inserted = diesel::insert_or_ignore_into(sync_acks::table)
        .values(&ack)
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(inserted > 0)
}

pub fn is_event_seen(conn: &mut SqliteConnection, event_id: &str) -> Result<bool, String> {
    let count: i64 = sync_seen::table
        .filter(sync_seen::event_id.eq(event_id))
        .count()
        .get_result(conn)
        .map_err(|e| e.to_string())?;

    Ok(count > 0)
}

pub fn mark_event_seen(conn: &mut SqliteConnection, event_id: &str) -> Result<(), String> {
    let seen = CreateSyncSeen {
        event_id: event_id.to_string(),
        received_at: utc_now(),
    };

    diesel::insert_or_ignore_into(sync_seen::table)
        .values(&seen)
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::open_db_at_path;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fini-test-replay-{label}-{}.db", Uuid::new_v4()))
    }

    #[test]
    fn ack_is_idempotent() {
        let db_path = temp_db_path("ack-idempotent");
        let mut conn = open_db_at_path(&db_path);

        record_ack(&mut conn, "peer-a", "event-1").expect("first ack");
        record_ack(&mut conn, "peer-a", "event-1").expect("duplicate ack");

        let count: i64 = sync_acks::table
            .filter(sync_acks::peer_device_id.eq("peer-a"))
            .filter(sync_acks::event_id.eq("event-1"))
            .count()
            .get_result(&mut conn)
            .expect("count acks");

        assert_eq!(count, 1);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn seen_tracking_works() {
        let db_path = temp_db_path("seen-tracking");
        let mut conn = open_db_at_path(&db_path);

        assert!(!is_event_seen(&mut conn, "event-1").unwrap());
        mark_event_seen(&mut conn, "event-1").expect("mark seen");
        assert!(is_event_seen(&mut conn, "event-1").unwrap());

        // idempotent
        mark_event_seen(&mut conn, "event-1").expect("mark seen again");
        assert!(is_event_seen(&mut conn, "event-1").unwrap());

        let _ = std::fs::remove_file(db_path);
    }
}
