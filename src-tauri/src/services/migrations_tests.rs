//! Tests that are about one migration in particular.
//!
//! Kept out of `db`, which is the service for opening and using the
//! database and should not accumulate knowledge of which migrations exist.
//! A migration's test belongs beside the other migrations' tests, not
//! inside the code that happens to run them.
//!
//! Worth testing here: a migration that carries *logic* -- a backfill, a
//! transform, defaults derived from what was there before. Getting one of
//! those wrong is invisible until a real person upgrades, and their data
//! is not recoverable afterwards. A migration that only adds a column has
//! nothing of ours to check.

use diesel::prelude::*;
use diesel_migrations::MigrationHarness;

use crate::schema::channels;
use crate::services::db::{open_db_at_path, temp_db_path, MIGRATIONS};

/// Migration 25 (ADR-0008 D14) turns every unlinked channel back into an
/// absent one and keeps every other channel exactly as it was.
///
/// The damage to guard against is the reverse: deleting a working channel
/// takes it away from a pair in the field on first launch.
#[test]
fn hard_unlink_migration_drops_only_unlinked_channels() {
    let db_path = temp_db_path("hard-unlink-drops-only-unlinked");
    let mut conn = open_db_at_path(&db_path);

    // Wind back until the tombstone column is back -- asked of the schema,
    // so this keeps meaning "before migration 25" however many land after it.
    while diesel::sql_query("SELECT unlinked_at FROM channels LIMIT 1")
        .execute(&mut conn)
        .is_err()
    {
        conn.revert_last_migration(MIGRATIONS)
            .expect("wind back past the hard-unlink migration");
    }

    diesel::sql_query(
        "INSERT INTO paired_devices (peer_device_id, display_name, paired_at, pair_state)
         VALUES ('upgraded-pair', 'Phone', '2026-01-01T00:00:00Z', 'paired')",
    )
    .execute(&mut conn)
    .expect("seed a pair");
    diesel::sql_query(
        "INSERT INTO channels (device_id, channel_kind, enabled, is_primary, address, configured_at, unlinked_at)
         VALUES ('upgraded-pair', 'network', 1, 1, NULL, '2026-01-01T00:00:00Z', NULL),
                ('upgraded-pair', 'bluetooth', 0, 0, NULL, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z')",
    )
    .execute(&mut conn)
    .expect("seed a linked and an unlinked channel");

    conn.run_pending_migrations(MIGRATIONS)
        .expect("upgrade a database that had a tombstoned channel");

    let rows: Vec<(String, bool, bool)> = channels::table
        .select((channels::channel_kind, channels::enabled, channels::is_primary))
        .load(&mut conn)
        .expect("load the upgraded channels");
    assert_eq!(
        rows,
        vec![("network".to_string(), true, true)],
        "the linked channel survives untouched, its primary choice included; the unlinked one is gone"
    );

    let _ = std::fs::remove_file(db_path);
}

/// Migration 26 (ADR-0008 D14): frames kept for a peer go with the pairing.
/// Without the cascade, a pair unpaired and paired again would receive the
/// old pairing's unlinks and sync ends.
#[test]
fn kept_peer_frames_go_with_the_pairing() {
    let db_path = temp_db_path("control-outbox-cascade");
    let mut conn = open_db_at_path(&db_path);

    // Wind back until the table is gone, so the upgrade itself creates it.
    while diesel::sql_query("SELECT id FROM peer_control_outbox LIMIT 1")
        .execute(&mut conn)
        .is_ok()
    {
        conn.revert_last_migration(MIGRATIONS)
            .expect("wind back past the control-outbox migration");
    }
    diesel::sql_query(
        "INSERT INTO paired_devices (peer_device_id, display_name, paired_at, pair_state)
         VALUES ('upgraded-pair', 'Phone', '2026-01-01T00:00:00Z', 'paired')",
    )
    .execute(&mut conn)
    .expect("seed a pair");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("upgrade to the control outbox");

    diesel::sql_query(
        "INSERT INTO peer_control_outbox (peer_device_id, frame_type, subject, frame, created_at)
         VALUES ('upgraded-pair', 'channel_unlinked', 'bluetooth', '{}', '2026-01-01T00:00:00Z')",
    )
    .execute(&mut conn)
    .expect("keep a frame for the pair");
    diesel::sql_query("DELETE FROM paired_devices WHERE peer_device_id = 'upgraded-pair'")
        .execute(&mut conn)
        .expect("unpair");

    let left: i64 = crate::schema::peer_control_outbox::table
        .count()
        .get_result(&mut conn)
        .expect("count kept frames");
    assert_eq!(left, 0, "unpairing drops what was kept for the pair");

    let _ = std::fs::remove_file(db_path);
}

/// Migration 27 (ADR-0009 D8): a pair made before keys existed keeps its
/// row, with no key pinned, so it is recognised and paired again rather than
/// trusted with whatever key connects first.
#[test]
fn a_pair_from_before_keys_upgrades_with_no_key_pinned() {
    let db_path = temp_db_path("endpoint-id-upgrade");
    let mut conn = open_db_at_path(&db_path);

    while diesel::sql_query("SELECT endpoint_id FROM paired_devices LIMIT 1")
        .execute(&mut conn)
        .is_ok()
    {
        conn.revert_last_migration(MIGRATIONS)
            .expect("wind back past the endpoint-id migration");
    }
    diesel::sql_query(
        "INSERT INTO paired_devices (peer_device_id, display_name, paired_at, pair_state)
         VALUES ('upgraded-pair', 'Phone', '2026-01-01T00:00:00Z', 'paired')",
    )
    .execute(&mut conn)
    .expect("seed a pair");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("upgrade to pinned keys");

    let pinned: Option<String> = crate::schema::paired_devices::table
        .find("upgraded-pair")
        .select(crate::schema::paired_devices::endpoint_id)
        .first(&mut conn)
        .expect("the pair survives the upgrade");
    assert_eq!(pinned, None);

    let _ = std::fs::remove_file(db_path);
}
