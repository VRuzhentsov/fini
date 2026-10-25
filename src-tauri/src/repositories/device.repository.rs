//! Every read and write of the `paired_devices` table: the devices this one
//! is paired with, and the key pinned for each (ADR-0009 D8).

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::{CreateDeviceInput, Device};
use crate::schema::paired_devices;
use crate::services::db::utc_now;

pub struct DeviceRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> DeviceRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// The paired devices, the newest pair first.
    pub fn list(&mut self) -> Result<Vec<Device>, String> {
        paired_devices::table
            .select(Device::as_select())
            .order(paired_devices::paired_at.desc())
            .load(&mut *self.conn)
            .map_err(|e| e.to_string())
    }

    pub fn find(&mut self, device_id: &str) -> Result<Option<Device>, String> {
        paired_devices::table
            .find(device_id)
            .select(Device::as_select())
            .first(&mut *self.conn)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Whether this device is paired with `device_id`. A read error reads as
    /// not paired.
    pub fn is_paired(&mut self, device_id: &str) -> bool {
        self.find(device_id).ok().flatten().is_some()
    }

    /// The ids of every paired device.
    pub fn ids(&mut self) -> Vec<String> {
        paired_devices::table
            .select(paired_devices::peer_device_id)
            .load(&mut *self.conn)
            .unwrap_or_default()
    }

    /// Records a pair, or updates its name when the pair exists, and pins
    /// `endpoint_id` when one is given: pairing again pins the key proved
    /// this time, because a reinstalled device has a new one.
    pub fn save(&mut self, device_id: &str, display_name: &str, endpoint_id: Option<&str>) -> Result<Device, String> {
        let now = utc_now();
        if self.find(device_id)?.is_some() {
            diesel::update(paired_devices::table.find(device_id))
                .set((paired_devices::display_name.eq(display_name), paired_devices::last_seen_at.eq(&now)))
                .execute(&mut *self.conn)
                .map_err(|e| e.to_string())?;
        } else {
            diesel::insert_into(paired_devices::table)
                .values(&CreateDeviceInput {
                    peer_device_id: device_id.to_string(),
                    display_name: display_name.to_string(),
                    paired_at: now,
                })
                .execute(&mut *self.conn)
                .map_err(|e| e.to_string())?;
        }
        if let Some(endpoint_id) = endpoint_id {
            diesel::update(paired_devices::table.find(device_id))
                .set(paired_devices::endpoint_id.eq(endpoint_id))
                .execute(&mut *self.conn)
                .map_err(|e| e.to_string())?;
        }
        self.find(device_id)?.ok_or_else(|| "the pair was not saved".to_string())
    }

    /// Removes the pair. Its channels and mappings go with it (the foreign
    /// keys cascade).
    pub fn remove(&mut self, device_id: &str) -> Result<(), String> {
        diesel::delete(paired_devices::table.find(device_id))
            .execute(&mut *self.conn)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn set_last_seen(&mut self, device_id: &str, last_seen_at: &str) -> Result<(), String> {
        diesel::update(paired_devices::table.find(device_id))
            .set(paired_devices::last_seen_at.eq(last_seen_at))
            .execute(&mut *self.conn)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// The iroh key pinned for this pair (ADR-0009 D8), if any.
    pub fn pinned_key(&mut self, device_id: &str) -> Option<String> {
        paired_devices::table
            .find(device_id)
            .select(paired_devices::endpoint_id)
            .first::<Option<String>>(&mut *self.conn)
            .ok()
            .flatten()
    }

    /// The paired device whose pinned key is `key`: who is on the other end
    /// of a connection whose TLS handshake proved `key` (ADR-0009 D8).
    pub fn find_by_key(&mut self, key: &str) -> Option<String> {
        paired_devices::table
            .filter(paired_devices::endpoint_id.eq(key))
            .select(paired_devices::peer_device_id)
            .first::<String>(&mut *self.conn)
            .ok()
    }

    /// Whether `link_key` is the key pinned for `device_id`. A link that
    /// proves no key, and a pair with no pinned key, both read as `false`.
    pub fn key_matches(&mut self, device_id: &str, link_key: Option<&str>) -> bool {
        link_key.is_some() && self.pinned_key(device_id).as_deref() == link_key
    }
}
