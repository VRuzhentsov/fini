//! Every read and write of the two outboxes and the acks.
//!
//! - `sync_outbox`: the changes this device made to quests in synced
//!   spaces, and `sync_acks`: which device has which change.
//! - `peer_control_outbox`: frames one device sends another on its own (a
//!   channel unlinked, a space no longer synced), kept until a session has
//!   carried them (ADR-0008 D14).

#[cfg(any(feature = "ui-plane", test))]
use chrono::Utc;
use diesel::prelude::*;
use diesel::SqliteConnection;
use uuid::Uuid;

use crate::models::{CreateSyncAck, CreateSyncOutboxEntry, CreateSyncSeen, SyncOutboxEntry};
#[cfg(any(feature = "ui-plane", test))]
use crate::models::{Quest, QuestOccurrence};
#[cfg(any(feature = "ui-plane", test))]
use crate::repositories::{quest_repository::QuestChange, QuestRepository};
#[cfg(any(feature = "ui-plane", test))]
use crate::schema::checklist_activity;
use crate::schema::{peer_control_outbox, sync_acks, sync_outbox, sync_seen};
use crate::services::channel_service::{PeerFrame, SyncEventEnvelope};
use crate::services::db::{sync_timestamp, utc_now};

const SPACE_SYNC_END: &str = "space_sync_end";

pub struct OutboxRepository<'a> {
    conn: &'a mut SqliteConnection,
}

fn envelope(row: SyncOutboxEntry) -> SyncEventEnvelope {
    SyncEventEnvelope {
        event_id: row.event_id,
        correlation_id: row.correlation_id,
        origin_device_id: row.origin_device_id,
        entity_type: row.entity_type,
        entity_id: row.entity_id,
        space_id: row.space_id,
        op_type: row.op_type,
        payload: row.payload,
        updated_at: row.updated_at,
        created_at: row.created_at,
    }
}

/// What a kept frame is about, so a later act on the same thing can remove
/// a frame that no longer holds. `None` for frames the outbox does not keep.
fn kept_as(frame: &PeerFrame) -> Option<(&'static str, String)> {
    match frame {
        PeerFrame::SpaceSyncEnd { space_id, .. } => Some((SPACE_SYNC_END, space_id.clone())),
        _ => None,
    }
}

impl<'a> OutboxRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// Records a change for the paired devices, made at `updated_at`.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_sync_event_at(
        &mut self,
        origin_device_id: &str,
        entity_type: &str,
        entity_id: &str,
        space_id: &str,
        op_type: &str,
        payload: Option<String>,
        updated_at: String,
    ) -> Result<(), String> {
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
        // Writes only. The command that made the change wakes SpaceSync
        // after its write, so that a tick reads the outbox with the row in it.
        diesel::insert_into(sync_outbox::table)
            .values(&entry)
            .execute(&mut *self.conn)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Records a change for the paired devices, made now.
    pub fn emit_sync_event(
        &mut self,
        origin_device_id: &str,
        entity_type: &str,
        entity_id: &str,
        space_id: &str,
        op_type: &str,
        payload: Option<String>,
    ) -> Result<(), String> {
        self.emit_sync_event_at(origin_device_id, entity_type, entity_id, space_id, op_type, payload, sync_timestamp())
    }

    /// The changes in these spaces that `peer_device_id` has not acked.
    pub fn load_unacked_events_for_peer(
        &mut self,
        peer_device_id: &str,
        mapped_space_ids: &[String],
    ) -> Result<Vec<SyncEventEnvelope>, String> {
        if mapped_space_ids.is_empty() {
            return Ok(vec![]);
        }
        let acked_event_ids: Vec<String> = sync_acks::table
            .filter(sync_acks::peer_device_id.eq(peer_device_id))
            .select(sync_acks::event_id)
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        let mut query = sync_outbox::table
            .filter(sync_outbox::space_id.eq_any(mapped_space_ids))
            .order(sync_outbox::created_at.asc())
            .into_boxed();
        if !acked_event_ids.is_empty() {
            query = query.filter(sync_outbox::event_id.ne_all(&acked_event_ids));
        }
        let rows: Vec<SyncOutboxEntry> = query
            .select(SyncOutboxEntry::as_select())
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(envelope).collect())
    }

    /// The newest change to one entity.
    pub fn load_latest_event_for_entity(
        &mut self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Option<SyncEventEnvelope>, String> {
        let row: Option<SyncOutboxEntry> = sync_outbox::table
            .filter(sync_outbox::entity_type.eq(entity_type))
            .filter(sync_outbox::entity_id.eq(entity_id))
            .order(sync_outbox::updated_at.desc())
            .select(SyncOutboxEntry::as_select())
            .first(&mut *self.conn)
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(row.map(envelope))
    }

    /// Every change in one space, oldest first.
    pub fn load_events_for_space(&mut self, space_id: &str) -> Result<Vec<SyncEventEnvelope>, String> {
        let rows: Vec<SyncOutboxEntry> = sync_outbox::table
            .filter(sync_outbox::space_id.eq(space_id))
            .order(sync_outbox::created_at.asc())
            .select(SyncOutboxEntry::as_select())
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(envelope).collect())
    }

    /// Records that `peer_device_id` has this change. `false` if it was
    /// recorded before.
    pub fn record_ack(&mut self, peer_device_id: &str, event_id: &str) -> Result<bool, String> {
        let inserted = diesel::insert_or_ignore_into(sync_acks::table)
            .values(&CreateSyncAck {
                peer_device_id: peer_device_id.to_string(),
                event_id: event_id.to_string(),
                acked_at: utc_now(),
            })
            .execute(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        Ok(inserted > 0)
    }

    /// Whether a change from a peer was applied here before.
    pub fn is_event_seen(&mut self, event_id: &str) -> Result<bool, String> {
        let count: i64 = sync_seen::table
            .filter(sync_seen::event_id.eq(event_id))
            .count()
            .get_result(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    }

    /// Records that a change from a peer was applied here.
    pub fn mark_event_seen(&mut self, event_id: &str) -> Result<(), String> {
        diesel::insert_or_ignore_into(sync_seen::table)
            .values(&CreateSyncSeen { event_id: event_id.to_string(), received_at: utc_now() })
            .execute(&mut *self.conn)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// The space and time of a change this device made, if it is in the
    /// outbox.
    pub fn space_and_time(&mut self, event_id: &str) -> Result<Option<(String, String)>, String> {
        sync_outbox::table
            .find(event_id)
            .select((sync_outbox::space_id, sync_outbox::updated_at))
            .first::<(String, String)>(&mut *self.conn)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Keeps `frame` for `peer_device_id` until a session carries it, in a
    /// transaction the caller owns, so the frame is written together with
    /// the change it reports. A frame about the same thing is replaced. Tell
    /// SpaceSync once the transaction commits (`SpaceSyncService::outbox_kept`).
    pub fn keep_in(
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        frame: &PeerFrame,
    ) -> Result<(), diesel::result::Error> {
        let (frame_type, subject) = kept_as(frame).ok_or_else(|| {
            diesel::result::Error::QueryBuilderError("this frame is not kept in the outbox".into())
        })?;
        let encoded =
            serde_json::to_string(frame).map_err(|e| diesel::result::Error::SerializationError(Box::new(e)))?;
        Self::remove_about(conn, peer_device_id, frame_type, &subject)?;
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
    }

    /// `keep_in`, in its own transaction.
    #[cfg(test)]
    pub fn keep(&mut self, peer_device_id: &str, frame: &PeerFrame) -> Result<(), String> {
        self.conn
            .transaction(|conn| Self::keep_in(conn, peer_device_id, frame))
            .map_err(|e: diesel::result::Error| e.to_string())
    }

    fn remove_about(
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

    /// The space is synced again: its end has not gone yet, so the peer
    /// never stopped, and delivered late it would undo the new request.
    pub fn drop_space_sync_end(&mut self, peer_device_id: &str, space_id: &str) -> Result<(), String> {
        Self::remove_about(self.conn, peer_device_id, SPACE_SYNC_END, space_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Frames waiting for `peer_device_id`, oldest first, with their row ids.
    pub fn waiting(&mut self, peer_device_id: &str) -> Vec<(i32, PeerFrame)> {
        let rows = peer_control_outbox::table
            .filter(peer_control_outbox::peer_device_id.eq(peer_device_id))
            .order(peer_control_outbox::id.asc())
            .select((peer_control_outbox::id, peer_control_outbox::frame))
            .load::<(i32, String)>(&mut *self.conn)
            .unwrap_or_default();
        let mut frames = Vec::new();
        for (id, frame) in rows {
            match serde_json::from_str(&frame) {
                Ok(frame) => frames.push((id, frame)),
                Err(err) => {
                    log::warn!("[control-outbox] removing unreadable frame {id}: {err}");
                    self.sent(id);
                }
            }
        }
        frames
    }

    /// A session wrote this frame to the link.
    pub fn sent(&mut self, id: i32) {
        let _ = diesel::delete(peer_control_outbox::table.find(id)).execute(&mut *self.conn);
    }
}


// ── Sync events ──────────────────────────────────────────────────────────────

/// Emits the stored quest row (never a resolved occurrence view: a repeating
/// quest syncs as one entity, ADR-0012 D11).
#[cfg(any(feature = "ui-plane", test))]
pub(crate) fn emit_quest_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    previous_space_id: &str,
    quest_id: &str,
) -> Result<(), String> {
    let quest = QuestRepository::new(conn).get(quest_id)?;
    let payload = serde_json::to_string(&quest).map_err(|e| e.to_string())?;

    if previous_space_id == quest.space_id {
        OutboxRepository::new(conn).emit_sync_event(
            origin_device_id,
            "quest",
            &quest.id,
            &quest.space_id,
            "upsert",
            Some(payload),
        )?;
        return emit_checklist_activity_sync_events(conn, origin_device_id, &quest);
    }

    let delete_updated_at = sync_timestamp();
    let upsert_updated_at = (Utc::now() + chrono::Duration::seconds(1))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    OutboxRepository::new(conn).emit_sync_event_at(
        origin_device_id,
        "quest",
        &quest.id,
        previous_space_id,
        "delete",
        None,
        delete_updated_at,
    )?;

    OutboxRepository::new(conn).emit_sync_event_at(
        origin_device_id,
        "quest",
        &quest.id,
        &quest.space_id,
        "upsert",
        Some(payload),
        upsert_updated_at,
    )?;

    emit_checklist_activity_sync_events(conn, origin_device_id, &quest)?;

    // The marks go with their quest to its new space.
    for occurrence in QuestRepository::new(conn).occurrences(&quest.id)? {
        emit_occurrence_sync_event(conn, origin_device_id, &quest.space_id, &occurrence)?;
    }

    Ok(())
}

/// Emits one occurrence mark: the entity `quest_occurrence`, keyed by
/// (quest_id, period_key) (ADR-0012 D11).
#[cfg(any(feature = "ui-plane", test))]
pub(crate) fn emit_occurrence_sync_event(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    space_id: &str,
    occurrence: &QuestOccurrence,
) -> Result<(), String> {
    let payload = serde_json::to_string(occurrence).map_err(|e| e.to_string())?;
    OutboxRepository::new(conn).emit_sync_event(
        origin_device_id,
        "quest_occurrence",
        &occurrence.sync_id(),
        space_id,
        "upsert",
        Some(payload),
    )
}

/// Emits a removed mark (a day opened again with nothing left to keep).
#[cfg(any(feature = "ui-plane", test))]
fn emit_occurrence_delete_event(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    space_id: &str,
    quest_id: &str,
    period_key: &str,
) -> Result<(), String> {
    OutboxRepository::new(conn).emit_sync_event(
        origin_device_id,
        "quest_occurrence",
        &format!("{quest_id}:{period_key}"),
        space_id,
        "delete",
        None,
    )
}

#[cfg(any(feature = "ui-plane", test))]
fn emit_checklist_activity_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    quest: &Quest,
) -> Result<(), String> {
    if !quest.is_checklist {
        return Ok(());
    }

    let emitted_activity_ids: Vec<String> = crate::schema::sync_outbox::table
        .filter(crate::schema::sync_outbox::entity_type.eq("checklist_activity"))
        .filter(crate::schema::sync_outbox::op_type.eq("upsert"))
        .filter(crate::schema::sync_outbox::space_id.eq(&quest.space_id))
        .select(crate::schema::sync_outbox::entity_id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let activities: Vec<crate::models::ChecklistActivity> = checklist_activity::table
        .filter(checklist_activity::quest_id.eq(&quest.id))
        .filter(
            checklist_activity::origin_device_id
                .is_null()
                .or(checklist_activity::origin_device_id.eq(origin_device_id)),
        )
        .filter(checklist_activity::id.ne_all(emitted_activity_ids))
        .select(crate::models::ChecklistActivity::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    for activity in activities {
        let payload = serde_json::to_string(&activity).map_err(|e| e.to_string())?;
        OutboxRepository::new(conn).emit_sync_event(
            origin_device_id,
            "checklist_activity",
            &activity.id,
            &quest.space_id,
            "upsert",
            Some(payload),
        )?;
    }

    Ok(())
}

/// Emits what a change wrote: the quest row, and each changed or removed mark.
#[cfg(any(feature = "ui-plane", test))]
pub(crate) fn emit_change_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    previous_space_id: &str,
    change: &QuestChange,
) -> Result<(), String> {
    if change.quest_row_changed {
        emit_quest_sync_events(conn, origin_device_id, previous_space_id, &change.quest.id)?;
    }
    let space_id = QuestRepository::new(conn).get(&change.quest.id)?.space_id;
    for occurrence in &change.occurrences {
        emit_occurrence_sync_event(conn, origin_device_id, &space_id, occurrence)?;
    }
    for period_key in &change.removed_occurrences {
        emit_occurrence_delete_event(conn, origin_device_id, &space_id, &change.quest.id, period_key)?;
    }
    if !change.quest_row_changed && !change.occurrences.is_empty() {
        // Checklist activity of a day is kept on the quest.
        let quest = QuestRepository::new(conn).get(&change.quest.id)?;
        emit_checklist_activity_sync_events(conn, origin_device_id, &quest)?;
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::open_db_at_path;

    #[test]
    fn emit_and_load_sync_events() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut conn = open_db_at_path(&dir.path().join("fini.db"));
        let mut outbox = OutboxRepository::new(&mut conn);
        outbox
            .emit_sync_event("device-a", "quest", "quest-1", "1", "upsert", Some(r#"{"title":"Test"}"#.to_string()))
            .expect("emit sync event");
        let events = outbox.load_unacked_events_for_peer("device-b", &["1".to_string()]).expect("load unacked");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_type, "quest");
        assert_eq!(events[0].entity_id, "quest-1");
        assert_eq!(events[0].op_type, "upsert");
    }
}
