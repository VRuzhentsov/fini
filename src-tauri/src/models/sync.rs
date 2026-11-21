use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{sync_acks, sync_outbox, sync_seen, tombstones};

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = sync_outbox)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct SyncOutboxEntry {
    pub event_id: String,
    pub correlation_id: String,
    pub origin_device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub op_type: String,
    pub payload: Option<String>,
    pub updated_at: String,
    pub created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sync_outbox)]
pub struct CreateSyncOutboxEntry {
    pub event_id: String,
    pub correlation_id: String,
    pub origin_device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub op_type: String,
    pub payload: Option<String>,
    pub updated_at: String,
}

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = sync_acks)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct SyncAck {
    pub peer_device_id: String,
    pub event_id: String,
    pub acked_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sync_acks)]
pub struct CreateSyncAck {
    pub peer_device_id: String,
    pub event_id: String,
    pub acked_at: String,
}

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = sync_seen)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct SyncSeen {
    pub event_id: String,
    pub received_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sync_seen)]
pub struct CreateSyncSeen {
    pub event_id: String,
    pub received_at: String,
}

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = tombstones)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Tombstone {
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub deleted_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = tombstones)]
pub struct CreateTombstone {
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
}
