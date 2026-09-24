//! Every read and write of the `channels` table.
//!
//! Nothing else in the crate touches it directly, because the interesting
//! rules are not in any one caller: a row existing means the channel is
//! configured, turning one off must release the primary, and recording a
//! diagnostic address must never bring a channel into existence. Those held
//! in three different places when this was six columns on `paired_devices`,
//! and disagreed.

use diesel::prelude::*;

use crate::models::channel::{Channel, NewChannel};
use crate::schema::channels;
use crate::services::db::utc_now;

use super::channel_status::ChannelKind;

/// Every channel this pair has configured, in a stable order (Network
/// first) so the Device page's rows never reorder under the reader.
pub fn configured(conn: &mut SqliteConnection, device_id: &str) -> Vec<Channel> {
    channels::table
        .filter(channels::device_id.eq(device_id))
        .filter(channels::unlinked_at.is_null())
        .select(Channel::as_select())
        .load(&mut *conn)
        .map(|mut rows: Vec<Channel>| {
            rows.sort_by_key(|row| row.channel_kind != ChannelKind::Network.code());
            rows
        })
        .unwrap_or_default()
}

pub fn find(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> Option<Channel> {
    row(conn, device_id, kind).filter(|channel| channel.unlinked_at.is_none())
}

/// The stored row whether or not it was unlinked.
///
/// Private, and used by exactly the three places that must see past the
/// tombstone: `configure`, which has to update the existing row rather than
/// insert a duplicate key; `unlink`, which writes it; and `ever_configured`,
/// which is what tells a peer's request apart from a first-time setup.
/// Everything else wants `find`, where an unlinked channel reads as absent
/// -- the meaning it had when unlinking deleted the row.
fn row(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> Option<Channel> {
    channels::table
        .find((device_id, kind.code()))
        .select(Channel::as_select())
        .first(&mut *conn)
        .optional()
        .ok()
        .flatten()
}

/// The kinds this pair has switched on, in `configured`'s stable order.
///
/// A failed read answers "none", like every other read here: callers ask
/// repeatedly rather than once, so a locked database costs a round instead
/// of an answer. A caller that cannot survive that distinction wants
/// `read_enabled`, which keeps it.
pub fn enabled_kinds(conn: &mut SqliteConnection, device_id: &str) -> Vec<ChannelKind> {
    configured(conn, device_id)
        .into_iter()
        .filter(|channel| channel.enabled)
        .filter_map(|channel| ChannelKind::from_code(&channel.channel_kind))
        .collect()
}

/// Set this channel up, switched on, only if the pair has never had it --
/// counting one it unlinked as having had it. `Ok(true)` if that created
/// the row, `Ok(false)` if something was already there.
///
/// What a peer is allowed to do (`PeerFrame::ChannelEnabled`): introduce a
/// channel this pair has never used, and nothing else.
///
/// One statement, deliberately. Asking `find`/`row` first and then calling
/// `configure` reads the table twice, and the first read answers `None` for
/// a transient `database is locked` exactly as it does for a row that
/// isn't there -- after which `configure` finds the row on its own second
/// read and switches it on, reversing the opt-out the check existed to
/// protect. `ON CONFLICT DO NOTHING` leaves whatever is there alone, and a
/// failure stays a failure instead of reading as permission.
pub fn introduce(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: ChannelKind,
) -> Result<bool, String> {
    let created = diesel::insert_into(channels::table)
        .values(&NewChannel {
            device_id: device_id.to_string(),
            channel_kind: kind.code().to_string(),
            enabled: true,
            is_primary: false,
            address: None,
            configured_at: utc_now(),
        })
        .on_conflict_do_nothing()
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(created > 0)
}

/// Whether this channel is configured *and* switched on. A missing row reads
/// as off, which is the direction every caller needs to fail in: an unknown
/// channel must not carry traffic.
pub fn is_enabled(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> bool {
    find(conn, device_id, kind).is_some_and(|channel| channel.enabled)
}

/// Whether this channel is switched on, distinguishing "off" from "could
/// not tell".
///
/// `is_enabled` answers `false` to both, which is the right direction for a
/// gate: an unknown channel must not carry traffic, and a refused
/// connection is retried a second later. It is the wrong direction for
/// anything that *destroys* something working -- a transient
/// `database is locked` then reads exactly like the person having flipped
/// the switch, and a healthy session is torn down for it.
///
/// `None` means the read itself failed. Callers that act destructively
/// must treat that as "leave it alone".
pub fn read_enabled(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: ChannelKind,
) -> Option<bool> {
    channels::table
        .find((device_id, kind.code()))
        .filter(channels::unlinked_at.is_null())
        .select(Channel::as_select())
        .first(conn)
        .optional()
        .ok()
        .map(|row| row.is_some_and(|channel| channel.enabled))
}

/// Whether the person set this channel up and then switched it off --
/// distinct from never having set it up, which is what made
/// `bluetooth_disabled_by_user` a separate column before rows were lazy.
pub fn is_switched_off(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> bool {
    find(conn, device_id, kind).is_some_and(|channel| !channel.enabled)
}

/// Set a channel up, or turn an existing one back on. Idempotent: calling it
/// for a channel that already exists updates the switch and, when one is
/// supplied, the address -- it never re-stamps `configured_at`, which
/// records when this pair first had this channel.
pub fn configure(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: ChannelKind,
    enabled: bool,
    address: Option<&str>,
) -> Result<(), String> {
    // `row`, not `find`: an unlinked channel still occupies the primary key,
    // so inserting over it would fail. Setting one up again is also the one
    // thing that should clear the tombstone -- the person is undoing their
    // own decision, which is exactly who is allowed to.
    if row(conn, device_id, kind).is_some() {
        diesel::update(channels::table.find((device_id, kind.code())))
            .set((
                channels::enabled.eq(enabled),
                channels::unlinked_at.eq(None::<String>),
            ))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
        if let Some(address) = address {
            diesel::update(channels::table.find((device_id, kind.code())))
                .set(channels::address.eq(address))
                .execute(&mut *conn)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }

    diesel::insert_into(channels::table)
        .values(&NewChannel {
            device_id: device_id.to_string(),
            channel_kind: kind.code().to_string(),
            enabled,
            is_primary: false,
            address: address.map(str::to_string),
            configured_at: utc_now(),
        })
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Turn a configured channel on or off. Turning it off also releases the
/// primary, in one transaction: a channel that is off and still primary
/// would suppress the other one while being unable to carry anything
/// itself, stranding the pair.
pub fn set_enabled(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: ChannelKind,
    enabled: bool,
) -> Result<(), String> {
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        diesel::update(channels::table.find((device_id, kind.code())))
            .set(channels::enabled.eq(enabled))
            .execute(conn)?;
        if !enabled {
            diesel::update(channels::table.find((device_id, kind.code())))
                .set(channels::is_primary.eq(false))
                .execute(conn)?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// Make one channel the primary, or clear the pair's primary entirely.
/// `channels_one_primary_per_device` makes at most one possible; clearing
/// first is what keeps the write from tripping it.
pub fn set_primary(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: Option<ChannelKind>,
) -> Result<(), String> {
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        diesel::update(channels::table.filter(channels::device_id.eq(device_id)))
            .set(channels::is_primary.eq(false))
            .execute(conn)?;
        if let Some(kind) = kind {
            diesel::update(channels::table.find((device_id, kind.code())))
                .set(channels::is_primary.eq(true))
                .execute(conn)?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// The channel the person chose to carry this pair's traffic, if they chose
/// one. `None` means automatic (network-first) selection.
pub fn primary_kind(conn: &mut SqliteConnection, device_id: &str) -> Option<ChannelKind> {
    channels::table
        .filter(channels::device_id.eq(device_id))
        .filter(channels::is_primary.eq(true))
        .select(channels::channel_kind)
        .first::<String>(&mut *conn)
        .ok()
        .and_then(|code| ChannelKind::from_code(&code))
}

/// Record where this channel last reached the peer, for diagnostics.
///
/// Updates an existing row and nothing else: creating one here would mean a
/// background observation silently configuring a channel the person never
/// set up, and a row that is switched off must stay exactly as they left it.
pub fn note_address(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind, address: &str) {
    let _ = diesel::update(
        channels::table
            .filter(channels::device_id.eq(device_id))
            .filter(channels::channel_kind.eq(kind.code()))
            .filter(channels::enabled.eq(true)),
    )
    .set(channels::address.eq(address))
    .execute(&mut *conn);
}

/// Forget a channel: the pair keeps its trust and its spaces, this channel
/// stops existing. Refused while it is on, so unlinking is always a
/// deliberate second act after switching off rather than something that can
/// happen to a working connection by one click.
pub fn unlink(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> Result<(), String> {
    let Some(channel) = find(conn, device_id, kind) else {
        return Ok(());
    };
    if channel.enabled {
        return Err("Turn the channel off first".to_string());
    }
    // Stamped, not deleted. The row reads as absent everywhere (`find` and
    // `configured` filter it out), so the page still offers to set the
    // channel up again -- but the decision survives, and a peer announcing
    // the same channel cannot undo it. See the migration's own note.
    diesel::update(channels::table.find((device_id, kind.code())))
        .set((
            channels::unlinked_at.eq(Some(utc_now())),
            channels::address.eq(None::<String>),
            channels::is_primary.eq(false),
        ))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Every pair with this channel configured and on -- the dial loops' candidate
/// list.
pub fn peers_with_channel_enabled(conn: &mut SqliteConnection, kind: ChannelKind) -> Vec<String> {
    channels::table
        .filter(channels::channel_kind.eq(kind.code()))
        .filter(channels::enabled.eq(true))
        .select(channels::device_id)
        .load(&mut *conn)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::open_db_at_path;

    /// The distinction the claim-time teardown depends on.
    ///
    /// `is_enabled` cannot tell "the person switched it off" from "I could
    /// not read the table", and a caller that closes a live session on the
    /// strength of that answer will close it for a blip. `read_enabled`
    /// keeps the two apart so such a caller can refuse to act on the second.
    #[test]
    fn read_enabled_separates_switched_off_from_unreadable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = open_db_at_path(&db_path);

        // `channels.device_id` references `paired_devices`, so the pair has
        // to exist before it can have a channel.
        diesel::sql_query(
            "INSERT INTO paired_devices (peer_device_id, display_name, paired_at) \
             VALUES ('peer', 'Peer', '2026-01-01T00:00:00Z')",
        )
        .execute(&mut conn)
        .expect("seed pair");

        // No row at all: definitely not on, and definitely readable.
        assert_eq!(read_enabled(&mut conn, "peer", ChannelKind::Bluetooth), Some(false));

        configure(&mut conn, "peer", ChannelKind::Bluetooth, true, None).expect("configure");
        assert_eq!(read_enabled(&mut conn, "peer", ChannelKind::Bluetooth), Some(true));

        set_enabled(&mut conn, "peer", ChannelKind::Bluetooth, false).expect("switch off");
        assert_eq!(read_enabled(&mut conn, "peer", ChannelKind::Bluetooth), Some(false));

        // And the case the whole thing exists for: a table that cannot be
        // read answers `None`, where `is_enabled` answers a confident and
        // wrong `false`.
        diesel::sql_query("DROP TABLE channels").execute(&mut conn).expect("drop");
        assert_eq!(read_enabled(&mut conn, "peer", ChannelKind::Bluetooth), None);
        assert!(!is_enabled(&mut conn, "peer", ChannelKind::Bluetooth));
    }

    /// What a peer is allowed to do to this pair's channels, in one
    /// statement: add one that was never here, and nothing else.
    ///
    /// The single statement is the point. Checking first and writing after
    /// reads the table twice, and the first read cannot tell "no row" from
    /// "could not read" -- so a lock held for a moment reads as permission,
    /// and the write then finds the row and switches it on. That is a local
    /// opt-out reversed by a transient error, which is why this is an
    /// insert that declines a conflict rather than a lookup.
    #[test]
    fn introduce_adds_only_a_channel_this_pair_never_had() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = open_db_at_path(&db_path);
        diesel::sql_query(
            "INSERT INTO paired_devices (peer_device_id, display_name, paired_at) \
             VALUES ('peer', 'Peer', '2026-01-01T00:00:00Z')",
        )
        .execute(&mut conn)
        .expect("seed pair");

        // Never had it: this is the one case that creates anything.
        assert_eq!(
            introduce(&mut conn, "peer", ChannelKind::Bluetooth),
            Ok(true)
        );
        assert!(is_enabled(&mut conn, "peer", ChannelKind::Bluetooth));

        // Switched off here: left alone.
        set_enabled(&mut conn, "peer", ChannelKind::Bluetooth, false).expect("switch off");
        assert_eq!(
            introduce(&mut conn, "peer", ChannelKind::Bluetooth),
            Ok(false)
        );
        assert!(!is_enabled(&mut conn, "peer", ChannelKind::Bluetooth));

        // Unlinked here: also left alone, and still absent to every reader.
        unlink(&mut conn, "peer", ChannelKind::Bluetooth).expect("unlink");
        assert_eq!(
            introduce(&mut conn, "peer", ChannelKind::Bluetooth),
            Ok(false)
        );
        assert!(find(&mut conn, "peer", ChannelKind::Bluetooth).is_none());
        assert!(!is_enabled(&mut conn, "peer", ChannelKind::Bluetooth));

        // And an unreadable table is an error, never a quiet `true` that
        // would let the caller write.
        diesel::sql_query("DROP TABLE channels").execute(&mut conn).expect("drop");
        assert!(introduce(&mut conn, "peer", ChannelKind::Bluetooth).is_err());
    }
}
