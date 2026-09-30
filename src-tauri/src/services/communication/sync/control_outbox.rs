//! Frames one device sends another on its own -- a channel unlinked, a space
//! no longer synced -- kept in the database until an exchange has carried
//! them (ADR-0008 D14). Either side may do these alone, so the peer must
//! learn of them however long it is away, across restarts and dropped
//! links.
//!
//! Sync events are not kept here: what to send is worked out again from the
//! two devices' state on every exchange. Requests that need the other person
//! at their device (a space sync request) are not kept either: they go now
//! or not at all.

use diesel::prelude::*;
use diesel::SqliteConnection;

use crate::schema::peer_control_outbox;
use crate::services::db::utc_now;

use super::types::PeerFrame;

const CHANNEL_UNLINKED: &str = "channel_unlinked";

/// Bumped whenever a frame is kept, so a running exchange sends it now
/// instead of leaving it for the next one.
pub fn changed() -> &'static tokio::sync::watch::Sender<u64> {
    static CHANGED: std::sync::OnceLock<tokio::sync::watch::Sender<u64>> = std::sync::OnceLock::new();
    CHANGED.get_or_init(|| tokio::sync::watch::channel(0).0)
}

const SPACE_SYNC_END: &str = "space_sync_end";

/// What a kept frame is about, so a later act on the same thing can drop a
/// frame that no longer holds. `None` for frames this outbox does not keep.
fn kept_as(frame: &PeerFrame) -> Option<(&'static str, String)> {
    match frame {
        PeerFrame::ChannelUnlinked { kind } => Some((CHANNEL_UNLINKED, kind.code().to_string())),
        PeerFrame::SpaceSyncEnd { space_id, .. } => Some((SPACE_SYNC_END, space_id.clone())),
        _ => None,
    }
}

/// Keeps `frame` for `peer_device_id` until an exchange carries it, and asks
/// for one. A frame about the same thing that has not gone yet is replaced.
pub fn keep(conn: &mut SqliteConnection, peer_device_id: &str, frame: &PeerFrame) -> Result<(), String> {
    let (frame_type, subject) =
        kept_as(frame).ok_or_else(|| "this frame is not kept in the outbox".to_string())?;
    let encoded = serde_json::to_string(frame).map_err(|e| e.to_string())?;
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        drop_about(conn, peer_device_id, frame_type, &subject)?;
        diesel::insert_into(peer_control_outbox::table)
            .values((
                peer_control_outbox::peer_device_id.eq(peer_device_id),
                peer_control_outbox::frame_type.eq(frame_type),
                peer_control_outbox::subject.eq(&subject),
                peer_control_outbox::frame.eq(encoded),
                peer_control_outbox::created_at.eq(utc_now()),
            ))
            .execute(conn)?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    changed().send_modify(|count| *count = count.wrapping_add(1));
    super::commands::notify_sync_work_pending();
    Ok(())
}

fn drop_about(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    frame_type: &str,
    subject: &str,
) -> Result<usize, diesel::result::Error> {
    diesel::delete(
        peer_control_outbox::table
            .filter(peer_control_outbox::peer_device_id.eq(peer_device_id))
            .filter(peer_control_outbox::frame_type.eq(frame_type))
            .filter(peer_control_outbox::subject.eq(subject)),
    )
    .execute(conn)
}

/// The channel was set up again: an unlink of it that has not gone yet no
/// longer holds, and delivered late it would delete the new channel.
pub fn drop_channel_unlinked(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    kind: crate::services::communication::pairing::ChannelKind,
) -> Result<(), String> {
    drop_about(conn, peer_device_id, CHANNEL_UNLINKED, kind.code())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The space is being synced again: its end has not gone yet, so the peer
/// never stopped, and delivered late it would undo the new request.
pub fn drop_space_sync_end(conn: &mut SqliteConnection, peer_device_id: &str, space_id: &str) -> Result<(), String> {
    drop_about(conn, peer_device_id, SPACE_SYNC_END, space_id)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Frames waiting for `peer_device_id`, oldest first, with their row ids.
pub fn waiting(conn: &mut SqliteConnection, peer_device_id: &str) -> Vec<(i32, PeerFrame)> {
    peer_control_outbox::table
        .filter(peer_control_outbox::peer_device_id.eq(peer_device_id))
        .order(peer_control_outbox::id.asc())
        .select((peer_control_outbox::id, peer_control_outbox::frame))
        .load::<(i32, String)>(conn)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, frame)| match serde_json::from_str(&frame) {
            Ok(frame) => Some((id, frame)),
            Err(err) => {
                log::warn!("[control-outbox] dropping unreadable frame {id}: {err}");
                let _ = diesel::delete(peer_control_outbox::table.find(id)).execute(conn);
                None
            }
        })
        .collect()
}

/// Peers with frames waiting: each needs an exchange.
pub fn peers_waiting(conn: &mut SqliteConnection) -> Vec<String> {
    peer_control_outbox::table
        .select(peer_control_outbox::peer_device_id)
        .distinct()
        .load::<String>(conn)
        .unwrap_or_default()
}

/// The exchange wrote this frame to the link.
pub fn sent(conn: &mut SqliteConnection, id: i32) {
    let _ = diesel::delete(peer_control_outbox::table.find(id)).execute(conn);
}
