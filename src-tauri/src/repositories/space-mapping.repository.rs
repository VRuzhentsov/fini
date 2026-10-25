//! Every read and write of the `pair_space_mappings` table: which spaces a
//! pair syncs, since when, and until when.

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::CreateSpaceMappingInput;
use crate::schema::pair_space_mappings as mappings;
use crate::services::db::utc_now;

pub struct SpaceMappingRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> SpaceMappingRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// The spaces this pair syncs now, in id order.
    pub fn active(&mut self, peer_device_id: &str) -> Result<Vec<String>, String> {
        mappings::table
            .filter(mappings::peer_device_id.eq(peer_device_id))
            .filter(mappings::end_of_sync_at.is_null())
            .order(mappings::space_id.asc())
            .select(mappings::space_id)
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())
    }

    /// Every space this pair has a row for, ended or not.
    pub fn all(&mut self, peer_device_id: &str) -> Result<Vec<String>, String> {
        mappings::table
            .filter(mappings::peer_device_id.eq(peer_device_id))
            .order(mappings::space_id.asc())
            .select(mappings::space_id)
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())
    }

    /// Each space of this pair with its last sync and its end:
    /// `(space_id, last_synced_at, end_of_sync_at)`.
    pub fn rows(&mut self, peer_device_id: &str) -> Result<Vec<(String, Option<String>, Option<String>)>, String> {
        mappings::table
            .filter(mappings::peer_device_id.eq(peer_device_id))
            .order(mappings::space_id.asc())
            .select((mappings::space_id, mappings::last_synced_at, mappings::end_of_sync_at))
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())
    }

    /// Whether this pair syncs this space now.
    pub fn is_active(&mut self, peer_device_id: &str, space_id: &str) -> Result<bool, String> {
        let count: i64 = mappings::table
            .filter(mappings::peer_device_id.eq(peer_device_id))
            .filter(mappings::space_id.eq(space_id))
            .filter(mappings::end_of_sync_at.is_null())
            .count()
            .get_result(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    }

    /// The spaces this pair syncs that were never synced yet: they need a
    /// bootstrap.
    pub fn never_synced(&mut self, peer_device_id: &str) -> Result<Vec<String>, String> {
        mappings::table
            .filter(mappings::peer_device_id.eq(peer_device_id))
            .filter(mappings::last_synced_at.is_null())
            .filter(mappings::end_of_sync_at.is_null())
            .select(mappings::space_id)
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())
    }

    /// Ends the sync of these spaces with this pair at `ended_at`.
    pub fn end(&mut self, peer_device_id: &str, space_ids: &[String], ended_at: &str) -> Result<(), String> {
        if space_ids.is_empty() {
            return Ok(());
        }
        diesel::update(
            mappings::table
                .filter(mappings::peer_device_id.eq(peer_device_id))
                .filter(mappings::space_id.eq_any(space_ids)),
        )
        .set(mappings::end_of_sync_at.eq(Some(ended_at.to_string())))
        .execute(&mut *self.conn)
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Syncs these spaces with this pair again, from now: rows that ended
    /// start again, and spaces without a row get one.
    pub fn start(&mut self, peer_device_id: &str, space_ids: &[String]) -> Result<(), String> {
        if space_ids.is_empty() {
            return Ok(());
        }
        let existing = self.all(peer_device_id)?;
        let (again, new): (Vec<String>, Vec<String>) =
            space_ids.iter().cloned().partition(|space_id| existing.contains(space_id));
        let now = utc_now();
        if !again.is_empty() {
            diesel::update(
                mappings::table
                    .filter(mappings::peer_device_id.eq(peer_device_id))
                    .filter(mappings::space_id.eq_any(&again)),
            )
            .set((
                mappings::enabled_at.eq(&now),
                mappings::last_synced_at.eq(Option::<String>::None),
                mappings::end_of_sync_at.eq(Option::<String>::None),
            ))
            .execute(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        }
        if !new.is_empty() {
            let rows: Vec<CreateSpaceMappingInput> = new
                .into_iter()
                .map(|space_id| CreateSpaceMappingInput {
                    peer_device_id: peer_device_id.to_string(),
                    space_id,
                    enabled_at: now.clone(),
                    last_synced_at: None,
                    end_of_sync_at: None,
                })
                .collect();
            diesel::insert_or_ignore_into(mappings::table)
                .values(&rows)
                .execute(&mut *self.conn)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Records that this pair synced this space up to `synced_at`, unless
    /// a later sync is recorded already.
    pub fn record_synced(&mut self, peer_device_id: &str, space_id: &str, synced_at: &str) -> Result<(), String> {
        let current: Option<String> = mappings::table
            .filter(mappings::peer_device_id.eq(peer_device_id))
            .filter(mappings::space_id.eq(space_id))
            .select(mappings::last_synced_at)
            .first::<Option<String>>(&mut *self.conn)
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        if current.as_deref().is_some_and(|current| current >= synced_at) {
            return Ok(());
        }
        self.set_last_synced(peer_device_id, space_id, synced_at)
    }

    /// Sets when this pair last synced this space.
    pub fn set_last_synced(&mut self, peer_device_id: &str, space_id: &str, synced_at: &str) -> Result<(), String> {
        diesel::update(
            mappings::table
                .filter(mappings::peer_device_id.eq(peer_device_id))
                .filter(mappings::space_id.eq(space_id)),
        )
        .set(mappings::last_synced_at.eq(Some(synced_at.to_string())))
        .execute(&mut *self.conn)
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// The pairs that have a row for this space.
    pub fn peers_of_space(&mut self, space_id: &str) -> Result<Vec<String>, String> {
        mappings::table
            .filter(mappings::space_id.eq(space_id))
            .select(mappings::peer_device_id)
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())
    }

    /// Moves every row of space `from` to space `to`.
    pub fn move_space(&mut self, from: &str, to: &str) -> Result<(), String> {
        for peer_device_id in self.peers_of_space(from)? {
            diesel::insert_or_ignore_into(mappings::table)
                .values((
                    mappings::peer_device_id.eq(&peer_device_id),
                    mappings::space_id.eq(to),
                    mappings::enabled_at.eq(utc_now()),
                ))
                .execute(&mut *self.conn)
                .map_err(|e| e.to_string())?;
        }
        diesel::delete(mappings::table.filter(mappings::space_id.eq(from)))
            .execute(&mut *self.conn)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
