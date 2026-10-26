//! Every read and write of the `channels` table.
//!
//! Nothing else in the crate touches it directly, because the interesting
//! rules are not in any one caller: a row existing means the channel is
//! configured, turning one off must release the primary, and recording a
//! diagnostic address must never bring a channel into existence.

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::channel::{Channel, ChannelKind, NewChannel};
use crate::schema::channels;
use crate::services::db::utc_now;

pub struct ChannelRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> ChannelRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// This pair's channel of this kind: `None` if it has none, otherwise
    /// `Off` or `On` by its `enabled` switch (ADR-0008 D15).
    pub fn find(&mut self, device_id: &str, kind: ChannelKind) -> Option<Channel> {
        channels::table
            .find((device_id, kind.code()))
            .select(Channel::as_select())
            .first(&mut *self.conn)
            .optional()
            .ok()
            .flatten()
    }

    /// Whether this channel is configured *and* switched on. A missing row
    /// reads as off: an unknown channel must not carry traffic.
    pub fn is_enabled(&mut self, device_id: &str, kind: ChannelKind) -> bool {
        self.find(device_id, kind).is_some_and(|channel| channel.enabled)
    }

    /// Set a channel up, or turn an existing one back on. Idempotent: it
    /// updates the switch and, when one is given, the address. It never
    /// changes `configured_at`, the time this pair first had this channel.
    pub fn configure(
        &mut self,
        device_id: &str,
        kind: ChannelKind,
        enabled: bool,
        address: Option<&str>,
    ) -> Result<(), String> {
        if self.find(device_id, kind).is_some() {
            diesel::update(channels::table.find((device_id, kind.code())))
                .set(channels::enabled.eq(enabled))
                .execute(&mut *self.conn)
                .map_err(|e| e.to_string())?;
            if let Some(address) = address {
                diesel::update(channels::table.find((device_id, kind.code())))
                    .set(channels::address.eq(address))
                    .execute(&mut *self.conn)
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
            .execute(&mut *self.conn)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Turn an existing channel on or off (ADR-0008 D15). Turning it off
    /// also releases the primary, in one transaction (ADR-0007).
    pub fn set_enabled(&mut self, device_id: &str, kind: ChannelKind, enabled: bool) -> Result<(), String> {
        self.conn
            .transaction::<(), diesel::result::Error, _>(|conn| {
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

    /// Make one channel the primary, or clear the pair's primary (ADR-0007).
    /// Clearing first keeps the write from tripping the one-primary index.
    pub fn set_primary(&mut self, device_id: &str, kind: Option<ChannelKind>) -> Result<(), String> {
        self.conn
            .transaction::<(), diesel::result::Error, _>(|conn| {
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

    /// The channel the person chose to carry this pair's traffic, if they
    /// chose one. `None` means Network first.
    pub fn primary_kind(&mut self, device_id: &str) -> Option<ChannelKind> {
        channels::table
            .filter(channels::device_id.eq(device_id))
            .filter(channels::is_primary.eq(true))
            .select(channels::channel_kind)
            .first::<String>(&mut *self.conn)
            .ok()
            .and_then(|code| ChannelKind::from_code(&code))
    }

    /// Record where this channel last reached the peer, for diagnostics.
    /// Updates an existing row that is on, and nothing else.
    pub fn note_address(&mut self, device_id: &str, kind: ChannelKind, address: &str) {
        let _ = diesel::update(
            channels::table
                .filter(channels::device_id.eq(device_id))
                .filter(channels::channel_kind.eq(kind.code()))
                .filter(channels::enabled.eq(true)),
        )
        .set(channels::address.eq(address))
        .execute(&mut *self.conn);
    }

    /// Every pair with this channel configured and on.
    pub fn peers_with_channel_enabled(&mut self, kind: ChannelKind) -> Vec<String> {
        channels::table
            .filter(channels::channel_kind.eq(kind.code()))
            .filter(channels::enabled.eq(true))
            .select(channels::device_id)
            .load(&mut *self.conn)
            .unwrap_or_default()
    }
}
