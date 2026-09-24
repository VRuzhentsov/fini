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

/// `channels.unlinked_at` is what lets a channel someone unlinked stay
/// unlinked when the peer asks for it back (#179).
///
/// The thing to prove is the direction of its default on an existing
/// database. Every channel already out there predates the column, so it
/// arrives NULL -- and NULL has to mean *linked*. The opposite reading
/// would take every pair's working channels away on first launch, which
/// is exactly the class of damage that is unrecoverable by the time
/// anyone notices.
#[test]
fn an_upgraded_database_keeps_every_channel_it_already_had() {
    let db_path = temp_db_path("upgrade-keeps-existing-channels-linked");
    let mut conn = open_db_at_path(&db_path);

    diesel::sql_query(
        "INSERT INTO paired_devices (peer_device_id, display_name, paired_at, pair_state)
         VALUES ('upgraded-pair', 'Phone', '2026-01-01T00:00:00Z', 'paired')",
    )
    .execute(&mut conn)
    .expect("seed a pair");
    diesel::sql_query(
        "INSERT INTO channels (device_id, channel_kind, enabled, is_primary, address, configured_at)
         VALUES ('upgraded-pair', 'network', 1, 1, NULL, '2026-01-01T00:00:00Z')",
    )
    .execute(&mut conn)
    .expect("seed a channel");

    // Wind back until the column is gone -- asked of the schema rather
    // than counted in migrations, so this keeps meaning "before the
    // tombstone" however many land after it.
    while channels::table
        .select(channels::unlinked_at)
        .limit(1)
        .load::<Option<String>>(&mut conn)
        .is_ok()
    {
        conn.revert_last_migration(MIGRATIONS)
            .expect("wind back past the tombstone column");
    }
    conn.run_pending_migrations(MIGRATIONS)
        .expect("upgrade a database that already had channels");

    let rows: Vec<(String, bool, Option<String>)> = channels::table
        .select((
            channels::channel_kind,
            channels::enabled,
            channels::unlinked_at,
        ))
        .load(&mut conn)
        .expect("load the upgraded channels");

    assert_eq!(
        rows,
        vec![("network".to_string(), true, None)],
        "a channel that predates the column keeps its switch and reads as linked -- anything \
         else takes working channels away from every pair in the field on first launch"
    );

    let _ = std::fs::remove_file(db_path);
}
