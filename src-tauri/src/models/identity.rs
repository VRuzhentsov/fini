//! This device's identity: its id, its name and its iroh key (ADR-0010
//! D15). The app start reads it first and gives it to the services as
//! values. It is stored in the settings rows `device.id`, `device.name` and
//! `device.secret_key`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::services::{db, settings};

const DEVICE_ID_KEY: &str = "device.id";
const DEVICE_NAME_KEY: &str = "device.name";
const SECRET_KEY_SETTING: &str = "device.secret_key";

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

/// The shape of the old `device_identity.json` file, read once to move its
/// id into the settings.
#[derive(Deserialize)]
struct FileIdentity {
    device_id: String,
}

impl Identity {
    /// This device's identity from the settings table, made the first time.
    /// An id from the old `device_identity.json` file is moved into the
    /// settings and the file is deleted.
    pub fn load_or_create(app_data_dir: &Path, db_path: &Path) -> Result<Self, String> {
        let file_identity_path = app_data_dir.join("device_identity.json");
        let mut conn = db::try_open_db_at_path(db_path)?;

        let settings_device_id = settings::load_setting(&mut conn, DEVICE_ID_KEY)
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty());
        let file_device_id = || {
            std::fs::read_to_string(&file_identity_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<FileIdentity>(&raw).ok())
                .map(|identity| identity.device_id)
                .filter(|value| !value.trim().is_empty())
        };

        let device_id = settings_device_id
            .or_else(file_device_id)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing_name = settings::load_setting(&mut conn, DEVICE_NAME_KEY)
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty());
        let hostname = current_device_name(&device_id, existing_name.as_deref());

        let _ = settings::upsert_setting(&mut conn, DEVICE_ID_KEY, &device_id);
        let _ = settings::upsert_setting(&mut conn, DEVICE_NAME_KEY, &hostname);
        let _ = std::fs::remove_file(file_identity_path);

        let secret_key = load_or_create_secret_key(&mut conn)?;
        Ok(Self {
            device_id,
            hostname,
            endpoint_id: secret_key.public().to_string(),
            secret_key,
        })
    }
}

fn load_or_create_secret_key(conn: &mut diesel::SqliteConnection) -> Result<iroh::SecretKey, String> {
    let stored = settings::load_setting(conn, SECRET_KEY_SETTING)
        .ok()
        .flatten()
        .and_then(|hex| hex.parse::<iroh::SecretKey>().ok());
    if let Some(key) = stored {
        return Ok(key);
    }
    let key = iroh::SecretKey::generate();
    let hex: String = key.to_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
    settings::upsert_setting(conn, SECRET_KEY_SETTING, &hex)
        .map_err(|err| format!("storing the device key failed: {err}"))?;
    Ok(key)
}

fn current_device_name(device_id: &str, existing_name: Option<&str>) -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| existing_name.map(ToString::to_string))
        .unwrap_or_else(|| format!("fini-{}", device_id.chars().take(8).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fini-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn load(dir: &Path) -> Identity {
        Identity::load_or_create(dir, &dir.join("fini.db")).expect("load or create identity")
    }

    #[test]
    fn the_identity_is_created_once_and_then_kept() {
        let dir = unique_temp_dir("identity-create");
        let first = load(&dir);
        let second = load(&dir);
        assert!(!first.device_id.trim().is_empty());
        assert_eq!(first.device_id, second.device_id);
        assert_eq!(first.secret_key.public(), second.secret_key.public());
        assert_eq!(first.endpoint_id, first.secret_key.public().to_string());
    }

    #[test]
    fn an_id_from_the_old_file_is_moved_into_the_settings() {
        let dir = unique_temp_dir("identity-import");
        std::fs::write(dir.join("device_identity.json"), r#"{"device_id":"file-id","hostname":"h"}"#)
            .expect("write the old file");
        assert_eq!(load(&dir).device_id, "file-id");
        assert!(!dir.join("device_identity.json").exists());
    }

    #[test]
    fn the_settings_id_wins_over_the_old_file() {
        let dir = unique_temp_dir("identity-settings-win");
        let mut conn = db::open_db_at_path(&dir.join("fini.db"));
        settings::upsert_setting(&mut conn, DEVICE_ID_KEY, "settings-id").expect("seed id");
        std::fs::write(dir.join("device_identity.json"), r#"{"device_id":"file-id"}"#)
            .expect("write the old file");
        assert_eq!(load(&dir).device_id, "settings-id");
    }
}
