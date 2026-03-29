use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::paired_devices;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = paired_devices)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct PairedDevice {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub pair_state: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = paired_devices)]
pub struct CreatePairedDeviceInput {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
}
