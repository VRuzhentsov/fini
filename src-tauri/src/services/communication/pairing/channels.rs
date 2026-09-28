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
            is_primary: false,
            address: address.map(str::to_string),
            configured_at: utc_now(),
        })
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Turn an existing channel on or off (ADR-0008 D15). Turning it off also
/// releases the primary, in one transaction: a channel that is off and
/// still primary would be preferred while unable to carry anything
/// (ADR-0007).
#[cfg(any(feature = "ui-plane", test))]
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

/// Make one channel the primary, or clear the pair's primary entirely
/// (ADR-0007). `channels_one_primary_per_device` allows at most one;
/// clearing first is what keeps the write from tripping it.
#[cfg(any(feature = "ui-plane", test))]
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
/// one. `None` means automatic (Network first) selection.
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
/// goes back to `None` (ADR-0008 D14). Refused while it is on, so unlinking
/// is always a deliberate second act after switching off rather than
/// something that can happen to a working connection by one click.
///
/// The caller pushes `ChannelUnlinked` to the peer so it removes its own
/// side too. Creating the channel again takes a mutual init, which the peer
/// cannot start on its own -- that, not a tombstone, is what keeps an
/// unlinked channel unlinked.
#[cfg(any(feature = "ui-plane", test))]
pub fn unlink(conn: &mut SqliteConnection, device_id: &str, kind: ChannelKind) -> Result<(), String> {
    let Some(channel) = find(conn, device_id, kind) else {
        return Ok(());
    };
    if channel.enabled {
        return Err("Turn the channel off first".to_string());
    }
    diesel::delete(channels::table.find((device_id, kind.code())))
        .execute(&mut *conn)
        .map(|_| ())
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

    /// ADR-0008 D14: unlinking removes the row, and only once the
    /// channel is off.
    #[test]
    fn unlink_deletes_the_channel_once_it_is_off() {
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
    }
}
