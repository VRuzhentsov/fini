use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::paired_devices;

#[derive(Debug, Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = paired_devices)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct PairedDevice {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub pair_state: String,
    pub bluetooth_enabled: bool,
    pub bluetooth_address: Option<String>,
    pub bluetooth_last_verified_at: Option<String>,
    /// Set when the user explicitly turns Bluetooth *off* for this pair via
    /// the Device settings toggle, cleared when they explicitly turn it
    /// back on -- distinct from `bluetooth_enabled` itself, which
    /// `persist_bluetooth_address_and_maybe_enable` also flips off for
    /// unrelated reasons (an address that isn't OS-bonded, an inconclusive
    /// check). Without this separate flag, a later self-reported address
    /// update over an authenticated session would happily re-confirm the
    /// bond and turn Bluetooth back on, silently undoing an explicit
    /// disable the moment the peer reconnects (`specs/device-connect/README.md`'s
    /// disable contract).
    pub bluetooth_disabled_by_user: bool,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = paired_devices)]
pub struct CreatePairedDeviceInput {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
}
