use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::paired_devices;

/// One device this one is paired with. The pair's *trust* and nothing else:
/// how the two actually reach each other lives in `channels` (ADR-0007), one
/// row per configured channel, so adding a channel is a row rather than four
/// more columns here.
#[derive(Debug, Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = paired_devices)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Device {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub pair_state: String,
    /// The peer's iroh key, pinned at pairing (ADR-0009 D8).
    pub endpoint_id: Option<String>,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = paired_devices)]
pub struct CreateDeviceInput {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
}

/// This device: its id, its name and its iroh key (ADR-0010 D15), stored in
/// the settings rows `device.id`, `device.name` and `device.secret_key`
/// (`DeviceRepository::load_or_create_identity`). The app start reads it
/// first and gives it to the services as values.
#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    pub device_id: String,
    pub hostname: String,
    /// The public half of this device's iroh key, hex (ADR-0009 D8).
    pub endpoint_id: String,
    /// This device's iroh key. Its public half is `endpoint_id`.
    #[serde(skip)]
    pub secret_key: iroh::SecretKey,
}
