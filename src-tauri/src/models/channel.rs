use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::channels;

/// One configured channel for one pair. The row existing is what "configured"
/// means; see `migrations/00000000000023_channels/up.sql`.
#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = channels)]
#[diesel(primary_key(device_id, channel_kind))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Channel {
    pub device_id: String,
    /// `ChannelKind`'s serde form -- `"network"` or `"bluetooth"`, the same
    /// strings seeded into `channel_kinds`.
    pub channel_kind: String,
    pub enabled: bool,
    /// The person's choice of this channel to carry the pair's traffic
    /// (ADR-0007). At most one per pair.
    pub is_primary: bool,
    /// The link-layer address this channel last reached the peer at, kept
    /// for diagnostics only -- ADR-0006 dials nothing by address.
    pub address: Option<String>,
    pub configured_at: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = channels)]
pub struct NewChannel {
    pub device_id: String,
    pub channel_kind: String,
    pub enabled: bool,
    pub is_primary: bool,
    pub address: Option<String>,
    pub configured_at: String,
}
