//! This device's iroh key (ADR-0009 D8): an Ed25519 key pair, created once
//! and kept in `settings` next to `device.id`. Its public half is the
//! device's `EndpointId`, which peers pin when pairing and which TLS proves
//! on every connection.

use std::path::Path;

use iroh::SecretKey;

use crate::services::{db, settings};

const SECRET_KEY_SETTING: &str = "device.secret_key";

pub(crate) fn try_load_or_create_secret_key(db_path: &Path) -> Result<SecretKey, String> {
    let mut conn = db::try_open_db_at_path(db_path)?;
    let stored = settings::load_setting(&mut conn, SECRET_KEY_SETTING)
        .ok()
        .flatten()
        .and_then(|hex| hex.parse::<SecretKey>().ok());
    if let Some(key) = stored {
        return Ok(key);
    }
    let key = SecretKey::generate();
    settings::upsert_setting(&mut conn, SECRET_KEY_SETTING, &to_hex(&key.to_bytes()))
        .map_err(|err| format!("storing the device key failed: {err}"))?;
    Ok(key)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_created_once_and_then_kept() {
        let db_path = db::temp_db_path("device-key");
        let first = try_load_or_create_secret_key(&db_path).expect("create");
        let second = try_load_or_create_secret_key(&db_path).expect("load");
        assert_eq!(first.public(), second.public());
        let _ = std::fs::remove_file(db_path);
    }
}
