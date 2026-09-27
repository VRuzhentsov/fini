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
use crate::schema::{channel_unlink_notices, channels};
use crate::services::db::utc_now;

use super::channel_status::ChannelKind;

/// This pair's channel of this kind: `None` if it has none, otherwise `Off`
/// or `On` by its `enabled` switch (ADR-0008 D15).
pub fn find(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> Option<Channel> {
    channels::table
        .find((device_id, kind.code()))
        .select(Channel::as_select())
        .first(&mut *conn)
        .optional()
        .ok()
        .flatten()
}

/// Whether this channel is configured *and* switched on. A missing row reads
/// as off, which is the direction every caller needs to fail in: an unknown
/// channel must not carry traffic.
pub fn is_enabled(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> bool {
    find(conn, device_id, kind).is_some_and(|channel| channel.enabled)
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
    if find(conn, device_id, kind).is_some() {
        diesel::update(channels::table.find((device_id, kind.code())))
            .set(channels::enabled.eq(enabled))
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
            address: address.map(str::to_string),
            configured_at: utc_now(),
        })
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    // A new channel supersedes an unlink of the old one this device still
    // owed the peer: sent now, it would remove the channel both just set up.
    clear_unlink_notice(conn, device_id, kind);
    Ok(())
}

/// Turn an existing channel on or off (ADR-0008 D15).
#[cfg(any(feature = "ui-plane", test))]
pub fn set_enabled(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: ChannelKind,
    enabled: bool,
) -> Result<(), String> {
    diesel::update(channels::table.find((device_id, kind.code())))
        .set(channels::enabled.eq(enabled))
        .execute(&mut *conn)
        .map(|_| ())
        .map_err(|e| e.to_string())
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
/// goes back to `None` (ADR-0008 D14). Refused while it is on, so unlinking
/// is always a deliberate second act after switching off rather than
/// something that can happen to a working connection by one click.
///
/// Also queues a notice for the peer, in the same transaction, so it
/// removes its own side too. Creating the channel again takes a mutual
/// init, which the peer cannot start on its own -- that, not a tombstone,
/// is what keeps an unlinked channel unlinked.
#[cfg(any(feature = "ui-plane", test))]
pub fn unlink(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> Result<(), String> {
    let Some(channel) = find(conn, device_id, kind) else {
        return Ok(());
    };
    if channel.enabled {
        return Err("Turn the channel off first".to_string());
    }
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        diesel::delete(channels::table.find((device_id, kind.code()))).execute(conn)?;
        diesel::insert_into(channel_unlink_notices::table)
            .values((
                channel_unlink_notices::device_id.eq(device_id),
                channel_unlink_notices::channel_kind.eq(kind.code()),
                channel_unlink_notices::created_at.eq(utc_now()),
            ))
            .on_conflict_do_nothing()
            .execute(conn)?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// The peer unlinked this channel on its side: remove ours, whatever its
/// switch says. The pair is broken for this channel either way, and there is
/// no point searching for a peer that will refuse every exchange on it.
pub fn remove_unlinked_by_peer(
    conn: &mut SqliteConnection,
    device_id: &str,
    kind: ChannelKind,
) -> Result<(), String> {
    diesel::delete(channels::table.find((device_id, kind.code())))
        .execute(&mut *conn)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Unlink notices this device still owes the peer, oldest first.
pub fn pending_unlink_notices(conn: &mut SqliteConnection, device_id: &str) -> Vec<ChannelKind> {
    channel_unlink_notices::table
        .filter(channel_unlink_notices::device_id.eq(device_id))
        .order(channel_unlink_notices::created_at.asc())
        .select(channel_unlink_notices::channel_kind)
        .load::<String>(&mut *conn)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|code| ChannelKind::from_code(&code))
        .collect()
}

/// The peer acknowledged an unlink notice; stop owing it.
pub fn clear_unlink_notice(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) {
    let _ = diesel::delete(channel_unlink_notices::table.find((device_id, kind.code())))
        .execute(&mut *conn);
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

    /// ADR-0008 D14: unlinking removes the row and leaves a notice for the
    /// peer, and a notice the peer acknowledges is gone.
    #[test]
    fn unlink_deletes_the_channel_and_owes_the_peer_a_notice() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = open_db_at_path(&db_path);
        diesel::sql_query(
            "INSERT INTO paired_devices (peer_device_id, display_name, paired_at) \
             VALUES ('peer', 'Peer', '2026-01-01T00:00:00Z')",
        )
        .execute(&mut conn)
        .expect("seed pair");

        configure(&mut conn, "peer", ChannelKind::Bluetooth, true, None).expect("configure");
        assert!(
            unlink(&mut conn, "peer", ChannelKind::Bluetooth).is_err(),
            "an On channel is switched off before it can be unlinked"
        );

        set_enabled(&mut conn, "peer", ChannelKind::Bluetooth, false).expect("switch off");
        unlink(&mut conn, "peer", ChannelKind::Bluetooth).expect("unlink");
        assert!(find(&mut conn, "peer", ChannelKind::Bluetooth).is_none());
        assert_eq!(pending_unlink_notices(&mut conn, "peer"), vec![ChannelKind::Bluetooth]);

        clear_unlink_notice(&mut conn, "peer", ChannelKind::Bluetooth);
        assert!(pending_unlink_notices(&mut conn, "peer").is_empty());
    }

    /// A channel set up again supersedes the unlink this device still owed
    /// for the old one: sending it would remove the new channel.
    #[test]
    fn setting_a_channel_up_again_drops_the_unlink_still_owed_for_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = open_db_at_path(&db_path);
        diesel::sql_query(
            "INSERT INTO paired_devices (peer_device_id, display_name, paired_at) \
             VALUES ('peer', 'Peer', '2026-01-01T00:00:00Z')",
        )
        .execute(&mut conn)
        .expect("seed pair");

        configure(&mut conn, "peer", ChannelKind::Bluetooth, false, None).expect("configure");
        unlink(&mut conn, "peer", ChannelKind::Bluetooth).expect("unlink");
        assert_eq!(pending_unlink_notices(&mut conn, "peer"), vec![ChannelKind::Bluetooth]);

        configure(&mut conn, "peer", ChannelKind::Bluetooth, true, None).expect("set up again");
        assert!(pending_unlink_notices(&mut conn, "peer").is_empty());
    }
}
