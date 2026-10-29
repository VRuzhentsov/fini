//! Every read and write of the `paired_devices` table: the devices this one
//! is paired with, and the key pinned for each (ADR-0009 D8).

use std::path::Path;

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::Deserialize;
use uuid::Uuid;

use crate::models::{CreateDeviceInput, Device, Identity};
use crate::schema::paired_devices;
use crate::services::db::utc_now;
use crate::services::settings;

const DEVICE_ID_KEY: &str = "device.id";
const DEVICE_NAME_KEY: &str = "device.name";
const SECRET_KEY_SETTING: &str = "device.secret_key";

/// The shape of the old `device_identity.json` file, read once to move its
/// id into the settings.
#[derive(Deserialize)]
struct FileIdentity {
    device_id: String,
}

pub struct DeviceRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> DeviceRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// This device's identity from the settings rows, made the first time.
    /// An id from the old `device_identity.json` file in `app_data_dir` is
    /// moved into the settings, and the file is deleted.
    pub fn load_or_create_identity(&mut self, app_data_dir: &Path) -> Result<Identity, String> {
        let file_identity_path = app_data_dir.join("device_identity.json");
        let settings_device_id = self.setting(DEVICE_ID_KEY);
        let file_device_id = || {
            std::fs::read_to_string(&file_identity_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<FileIdentity>(&raw).ok())
                .map(|identity| identity.device_id)
                .filter(|value| !value.trim().is_empty())
        };
        let device_id = settings_device_id.or_else(file_device_id).unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing_name = self.setting(DEVICE_NAME_KEY);
        let hostname = Self::current_device_name(&device_id, existing_name.as_deref());

        let _ = settings::upsert_setting(self.conn, DEVICE_ID_KEY, &device_id);
        let _ = settings::upsert_setting(self.conn, DEVICE_NAME_KEY, &hostname);
        let _ = std::fs::remove_file(file_identity_path);

        let secret_key = self.load_or_create_secret_key()?;
        Ok(Identity { device_id, hostname, endpoint_id: secret_key.public().to_string(), secret_key })
    }

    /// A settings row, if it has a value that is not blank.
    fn setting(&mut self, key: &str) -> Option<String> {
        settings::load_setting(self.conn, key).ok().flatten().filter(|value| !value.trim().is_empty())
    }

    fn load_or_create_secret_key(&mut self) -> Result<iroh::SecretKey, String> {
        if let Some(key) = self.setting(SECRET_KEY_SETTING).and_then(|hex| hex.parse::<iroh::SecretKey>().ok()) {
            return Ok(key);
        }
        let key = iroh::SecretKey::generate();
        let hex: String = key.to_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
        settings::upsert_setting(self.conn, SECRET_KEY_SETTING, &hex)
            .map_err(|err| format!("storing the device key failed: {err}"))?;
        Ok(key)
    }

    /// The name of this device: the host name, else the name stored before,
    /// else one made from the id.
    fn current_device_name(device_id: &str, existing_name: Option<&str>) -> String {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| existing_name.map(ToString::to_string))
            .unwrap_or_else(|| format!("fini-{}", device_id.chars().take(8).collect::<String>()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::open_db_at_path;

    fn load(dir: &Path) -> Identity {
        DeviceRepository::new(&mut open_db_at_path(&dir.join("fini.db")))
            .load_or_create_identity(dir)
            .expect("load or create identity")
    }

    #[test]
    fn the_identity_is_created_once_and_then_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = load(dir.path());
        let second = load(dir.path());
        assert!(!first.device_id.trim().is_empty());
        assert_eq!(first.device_id, second.device_id);
        assert_eq!(first.secret_key.public(), second.secret_key.public());
        assert_eq!(first.endpoint_id, first.secret_key.public().to_string());
    }

    #[test]
    fn an_id_from_the_old_file_is_moved_into_the_settings() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("device_identity.json"), r#"{"device_id":"file-id","hostname":"h"}"#)
            .expect("write the old file");
        assert_eq!(load(dir.path()).device_id, "file-id");
        assert!(!dir.path().join("device_identity.json").exists());
    }

    #[test]
    fn the_settings_id_wins_over_the_old_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut conn = open_db_at_path(&dir.path().join("fini.db"));
        settings::upsert_setting(&mut conn, DEVICE_ID_KEY, "settings-id").expect("seed id");
        std::fs::write(dir.path().join("device_identity.json"), r#"{"device_id":"file-id"}"#)
            .expect("write the old file");
        assert_eq!(load(dir.path()).device_id, "settings-id");
    }
}
