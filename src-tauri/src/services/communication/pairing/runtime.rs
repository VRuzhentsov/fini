use chrono::Utc;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

#[cfg(test)]
use std::time::Instant;
#[cfg(test)]
use super::DISCOVERY_PROTOCOL;
use super::{DISCOVERY_TTL_SECS, NETWORK_CHANNEL_TIMEOUT};
#[cfg(any(feature = "ui-plane", test))]
use super::PAIR_REQUEST_TTL_SECS;
use crate::services::communication::pairing::types::{DeviceIdentity, DiscoveryRuntime};
#[cfg(test)]
use crate::services::communication::pairing::types::SeenPeer;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::pairing::types::{
    IncomingPairRequest, PairRequestPayload, StoredIncomingPairRequest,
};
use crate::services::{db, settings};

const DEVICE_ID_KEY: &str = "device.id";
const DEVICE_NAME_KEY: &str = "device.name";

pub(super) fn utc_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

impl DeviceIdentity {
    /// This device's identity from the settings table, made the first time.
    /// An id from the old `device_identity.json` file is moved into the
    /// settings and the file is deleted.
    pub(super) fn load_or_create(app_data_dir: &Path, db_path: &Path) -> Result<Self, String> {
        let file_identity_path = app_data_dir.join("device_identity.json");
        let mut conn = db::try_open_db_at_path(db_path)?;

        let settings_device_id = settings::load_setting(&mut conn, DEVICE_ID_KEY)
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty());
        let file_device_id = || {
            std::fs::read_to_string(&file_identity_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<DeviceIdentity>(&raw).ok())
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

        Ok(Self {
            device_id,
            hostname,
            endpoint_id: String::new(),
        })
    }
}

fn current_device_name(device_id: &str, existing_name: Option<&str>) -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| existing_name.map(ToString::to_string))
        .unwrap_or_else(|| format!("fini-{}", device_id.chars().take(8).collect::<String>()))
}

pub(super) fn parse_utc_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

pub(super) fn request_is_expired(expires_at: &str) -> bool {
    let Some(expires_at) = parse_utc_timestamp(expires_at) else {
        return true;
    };
    Utc::now() >= expires_at
}

impl DiscoveryRuntime {
    /// Keeps only the recent part of what the network discovery heard. A
    /// device not heard within the channel timeout is no longer present.
    /// The Add Device list keeps only devices heard in the last
    /// `DISCOVERY_TTL_SECS`, and only while this device is in Add Device
    /// mode.
    pub(super) fn keep_recent_network_records(&mut self) {
        let discovery_ttl = Duration::from_secs(DISCOVERY_TTL_SECS);
        self.discovered.retain(|_, peer| peer.last_seen_mono.elapsed() <= discovery_ttl);
        // A device that stopped announcing is no longer present, and its
        // next announcement counts as arriving anew.
        self.presence.retain(|_, peer| peer.last_seen_mono.elapsed() < NETWORK_CHANNEL_TIMEOUT);
        self.prune_expired_incoming_requests();

        if !self.add_mode_enabled {
            self.discovered.clear();
            self.incoming_requests.clear();
            self.outgoing_code_updates.clear();
            self.outgoing_pair_completions.clear();
            self.pairing_keys.clear();
        }
    }

    pub(super) fn prune_expired_incoming_requests(&mut self) {
        self.incoming_requests
            .retain(|_, request| !request_is_expired(&request.request.expires_at));
    }

    /// Whether a device's last network record is recent enough to count: an
    /// entry past the channel timeout is a device that went away, and its
    /// next record is an arrival, even before the entry is removed.
    pub(super) fn is_present(&self, device_id: &str) -> bool {
        self.presence
            .get(device_id)
            .is_some_and(|peer| peer.last_seen_mono.elapsed() < NETWORK_CHANNEL_TIMEOUT)
    }
}

pub(super) fn generate_passcode() -> String {
    let value = (Uuid::new_v4().as_u128() % 1_000_000) as u32;
    format!("{value:06}")
}

#[cfg(any(feature = "ui-plane", test))]
impl StoredIncomingPairRequest {
    /// A pair request as this device stores it when it arrives. Its expiry
    /// is counted from this device's clock, not the sender's.
    pub(super) fn from_payload(
        payload: &PairRequestPayload,
        from_addr: String,
        from_endpoint_id: Option<String>,
        via_bluetooth: bool,
    ) -> Self {
        let created_at = utc_now();
        let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        // When `via_bluetooth`, `from_addr` is the sender's Bluetooth address
        // as observed on this connection (`DataLink::peer_addr()`), not an IP
        // -- see `IncomingPairRequest::from_bluetooth_address`.
        let from_bluetooth_address = via_bluetooth.then(|| from_addr.clone());

        Self {
            request: IncomingPairRequest {
                request_id: payload.request_id.clone(),
                from_device_id: payload.from_device_id.clone(),
                from_hostname: payload.from_hostname.clone(),
                created_at,
                expires_at,
                attempts: 0,
                cooldown_until: None,
                via_bluetooth,
                from_bluetooth_address,
            },
            from_addr,
            from_ws_port: payload.from_ws_port,
            from_endpoint_id,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::communication::pairing::{DISCOVERY_PORT, SPACE_SYNC_WS_PORT};
    use chrono::Duration as ChronoDuration;
    use std::path::PathBuf;

    fn load_or_create_identity(app_data_dir: &Path) -> DeviceIdentity {
        DeviceIdentity::load_or_create(app_data_dir, &app_data_dir.join("fini.db"))
            .expect("failed to load or create device identity")
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fini-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn sample_pair_request_payload(expires_at: &str) -> PairRequestPayload {
        PairRequestPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_request".to_string(),
            request_id: "req-1".to_string(),
            from_device_id: "device-a".to_string(),
            from_hostname: "alpha".to_string(),
            from_discovery_port: Some(DISCOVERY_PORT),
            from_ws_port: Some(SPACE_SYNC_WS_PORT),
            to_device_id: "device-b".to_string(),
            created_at: "2000-01-01T00:00:00Z".to_string(),
            expires_at: expires_at.to_string(),
            from_endpoint_id: None,
        }
    }

    fn stored_request(request_id: &str, expires_at: &str) -> StoredIncomingPairRequest {
        StoredIncomingPairRequest {
            request: IncomingPairRequest {
                request_id: request_id.to_string(),
                from_device_id: "sender".to_string(),
                from_hostname: "sender-host".to_string(),
                created_at: utc_now(),
                expires_at: expires_at.to_string(),
                attempts: 0,
                cooldown_until: None,
                via_bluetooth: false,
                from_bluetooth_address: None,
            },
            from_addr: "127.0.0.1".to_string(),
            from_ws_port: Some(SPACE_SYNC_WS_PORT),
            from_endpoint_id: None,
        }
    }

    #[test]
    fn passcode_is_six_ascii_digits() {
        for _ in 0..128 {
            let code = generate_passcode();
            assert_eq!(code.len(), 6, "passcode must be 6 chars");
            assert!(
                code.chars().all(|ch| ch.is_ascii_digit()),
                "passcode must be numeric"
            );
        }
    }

    #[test]
    fn invalid_timestamp_is_treated_as_expired() {
        assert!(request_is_expired("not-a-valid-timestamp"));
    }

    #[test]
    fn future_timestamp_is_not_expired() {
        let future = (Utc::now() + ChronoDuration::seconds(30))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        assert!(!request_is_expired(&future));
    }

    #[test]
    fn parse_utc_timestamp_handles_offset_inputs() {
        let parsed = parse_utc_timestamp("2026-03-27T01:00:00+02:00")
            .expect("offset timestamp should parse");
        assert_eq!(parsed.to_rfc3339(), "2026-03-26T23:00:00+00:00");
    }

    #[test]
    fn prune_expired_incoming_requests_removes_only_expired_items() {
        let now = Utc::now();
        let past = (now - ChronoDuration::seconds(10))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let future = (now + ChronoDuration::seconds(120))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let mut runtime = DiscoveryRuntime::default();
        runtime
            .incoming_requests
            .insert("expired".to_string(), stored_request("expired", &past));
        runtime
            .incoming_requests
            .insert("active".to_string(), stored_request("active", &future));
        runtime
            .incoming_requests
            .insert("invalid".to_string(), stored_request("invalid", "bad-ts"));

        runtime.prune_expired_incoming_requests();

        assert!(runtime.incoming_requests.contains_key("active"));
        assert!(!runtime.incoming_requests.contains_key("expired"));
        assert!(!runtime.incoming_requests.contains_key("invalid"));
    }

    #[test]
    fn incoming_pair_request_uses_receiver_local_ttl() {
        let payload = sample_pair_request_payload("1999-01-01T00:00:00Z");
        let before = Utc::now();
        let stored = StoredIncomingPairRequest::from_payload(&payload, "192.168.1.50".to_string(), None, false);
        let after = Utc::now();

        assert_eq!(stored.request.request_id, payload.request_id);
        assert_eq!(stored.request.from_device_id, payload.from_device_id);
        assert_eq!(stored.request.from_hostname, payload.from_hostname);
        assert_eq!(stored.from_addr, "192.168.1.50");
        assert_eq!(stored.request.attempts, 0);
        assert_eq!(stored.request.cooldown_until, None);

        let created_at = parse_utc_timestamp(&stored.request.created_at)
            .expect("created_at should be valid UTC timestamp");
        let expires_at = parse_utc_timestamp(&stored.request.expires_at)
            .expect("expires_at should be valid UTC timestamp");

        let created_min = before - ChronoDuration::seconds(2);
        let created_max = after + ChronoDuration::seconds(2);
        assert!(created_at >= created_min && created_at <= created_max);

        let ttl_min = before + ChronoDuration::seconds(PAIR_REQUEST_TTL_SECS - 2);
        let ttl_max = after + ChronoDuration::seconds(PAIR_REQUEST_TTL_SECS + 2);
        assert!(
            expires_at >= ttl_min && expires_at <= ttl_max,
            "receiver expiry should be computed from local now"
        );
    }

    #[test]
    fn load_or_create_identity_creates_and_persists_settings_identity() {
        let dir = unique_temp_dir("identity-create");
        let identity = load_or_create_identity(&dir);

        assert!(!identity.device_id.trim().is_empty());
        assert!(!identity.hostname.trim().is_empty());

        let mut conn = db::open_db_at_path(&dir.join("fini.db"));
        let saved_id = settings::load_setting(&mut conn, DEVICE_ID_KEY)
            .expect("load device id setting")
            .expect("device id setting should exist");
        let saved_name = settings::load_setting(&mut conn, DEVICE_NAME_KEY)
            .expect("load device name setting")
            .expect("device name setting should exist");

        assert_eq!(saved_id, identity.device_id);
        assert_eq!(saved_name, identity.hostname);
        assert!(
            !dir.join("device_identity.json").exists(),
            "deprecated identity file should not be created"
        );
    }

    #[test]
    fn load_or_create_identity_imports_file_id_and_deletes_file() {
        let dir = unique_temp_dir("identity-import");
        let file_identity = DeviceIdentity {
            device_id: "existing-device-id".to_string(),
            hostname: "file-host".to_string(),
            endpoint_id: String::new(),
        };

        let payload =
            serde_json::to_string_pretty(&file_identity).expect("serialize fixture identity");
        std::fs::write(dir.join("device_identity.json"), payload)
            .expect("failed to write fixture identity file");

        let loaded = load_or_create_identity(&dir);
        assert_eq!(loaded.device_id, file_identity.device_id);
        assert!(!loaded.hostname.trim().is_empty());
        assert!(
            !dir.join("device_identity.json").exists(),
            "deprecated identity file should be deleted after import"
        );
    }

    #[test]
    fn load_or_create_identity_prefers_settings_over_file_identity() {
        let dir = unique_temp_dir("identity-settings-win");
        let mut conn = db::open_db_at_path(&dir.join("fini.db"));
        settings::upsert_setting(&mut conn, DEVICE_ID_KEY, "settings-device-id")
            .expect("seed device id setting");
        settings::upsert_setting(&mut conn, DEVICE_NAME_KEY, "settings-device-name")
            .expect("seed device name setting");
        let file_identity = DeviceIdentity {
            device_id: "file-device-id".to_string(),
            hostname: "file-host".to_string(),
            endpoint_id: String::new(),
        };
        let payload =
            serde_json::to_string_pretty(&file_identity).expect("serialize fixture identity");
        std::fs::write(dir.join("device_identity.json"), payload)
            .expect("failed to write fixture identity file");

        let loaded = load_or_create_identity(&dir);

        assert_eq!(loaded.device_id, "settings-device-id");
        assert!(!loaded.hostname.trim().is_empty());
        assert!(
            !dir.join("device_identity.json").exists(),
            "stale deprecated identity file should be deleted"
        );
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    fn seen(ago: Duration) -> SeenPeer {
        SeenPeer {
            hostname: "peer".to_string(),
            addr: "127.0.0.1".to_string(),
            discovery_port: 0,
            ws_port: None,
            endpoint_id: None,
            last_seen_at: crate::services::db::utc_now(),
            last_seen_mono: Instant::now() - ago,
        }
    }

    /// A beacon from a peer whose entry outlived the channel timeout is an
    /// arrival, pruned or not.
    #[test]
    fn a_stale_entry_does_not_count_as_present() {
        let mut runtime = DiscoveryRuntime::default();
        runtime.presence.insert("fresh".to_string(), seen(Duration::ZERO));
        runtime.presence.insert("stale".to_string(), seen(NETWORK_CHANNEL_TIMEOUT));
        assert!(runtime.is_present("fresh"));
        assert!(!runtime.is_present("stale"));
        assert!(!runtime.is_present("unknown"));
    }
}
