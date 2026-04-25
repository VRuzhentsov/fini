use chrono::Utc;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::{
    DISCOVERY_INTERVAL_MS, DISCOVERY_PORT, DISCOVERY_PROTOCOL, DISCOVERY_TTL_SECS,
    HEARTBEAT_INTERVAL_MS, MULTICAST_GROUP, PAIR_REQUEST_TTL_SECS, SPACE_SYNC_WS_PORT,
};
use crate::services::device_connection::types::{
    DeviceIdentity, DiscoveryBeacon, DiscoveryRuntime, IncomingPairRequest, PairAcceptPayload,
    PairCodeUpdate, PairCompletePayload, PairCompletionUpdate, PairRequestPayload, SeenPeer,
    StoredIncomingPairRequest,
};

pub(super) fn utc_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(super) fn load_or_create_identity(app_data_dir: &Path) -> DeviceIdentity {
    let path = app_data_dir.join("device_identity.json");

    if let Some(parsed) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<DeviceIdentity>(&raw).ok())
        .filter(|identity| {
            !identity.device_id.trim().is_empty() && !identity.hostname.trim().is_empty()
        })
    {
        return parsed;
    }

    let device_id = Uuid::new_v4().to_string();
    let fallback_host = format!("fini-{}", &device_id[..8]);
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_host);

    let identity = DeviceIdentity {
        device_id,
        hostname,
    };

    if let Ok(serialized) = serde_json::to_string_pretty(&identity) {
        let _ = std::fs::write(path, serialized);
    }

    identity
}

pub(super) fn set_last_error(runtime: &Arc<Mutex<DiscoveryRuntime>>, message: String) {
    if let Ok(mut guard) = runtime.lock() {
        guard.last_error = Some(message);
    }
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

pub(super) fn prune_expired_incoming_requests(runtime: &mut DiscoveryRuntime) {
    runtime
        .incoming_requests
        .retain(|_, request| !request_is_expired(&request.request.expires_at));
}

pub(super) fn generate_passcode() -> String {
    let value = (Uuid::new_v4().as_u128() % 1_000_000) as u32;
    format!("{value:06}")
}

pub(super) fn build_incoming_pair_request(
    payload: &PairRequestPayload,
    from_addr: String,
) -> StoredIncomingPairRequest {
    let created_at = utc_now();
    let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    StoredIncomingPairRequest {
        request: IncomingPairRequest {
            request_id: payload.request_id.clone(),
            from_device_id: payload.from_device_id.clone(),
            from_hostname: payload.from_hostname.clone(),
            created_at,
            expires_at,
            attempts: 0,
            cooldown_until: None,
        },
        from_addr,
    }
}

pub(super) fn upsert_seen_peer(
    peers: &mut HashMap<String, SeenPeer>,
    beacon: &DiscoveryBeacon,
    addr: IpAddr,
) {
    peers.insert(
        beacon.device_id.clone(),
        SeenPeer {
            hostname: beacon.hostname.clone(),
            addr: addr.to_string(),
            ws_port: beacon.ws_port,
            last_seen_at: utc_now(),
            last_seen_mono: Instant::now(),
        },
    );
}

fn broadcast_beacon(
    socket: &UdpSocket,
    runtime: &Arc<Mutex<DiscoveryRuntime>>,
    identity: &DeviceIdentity,
    mode: &str,
) {
    let beacon = DiscoveryBeacon {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        mode: mode.to_string(),
        device_id: identity.device_id.clone(),
        hostname: identity.hostname.clone(),
        sent_at: utc_now(),
        ws_port: Some(SPACE_SYNC_WS_PORT),
    };

    let payload = serde_json::to_vec(&beacon);
    match payload {
        Ok(body) => {
            let multicast_target = SocketAddr::from((MULTICAST_GROUP, DISCOVERY_PORT));
            let broadcast_target =
                SocketAddr::from((Ipv4Addr::new(255, 255, 255, 255), DISCOVERY_PORT));

            let send_multicast = socket.send_to(&body, multicast_target);
            let send_broadcast = socket.send_to(&body, broadcast_target);
            let sent = send_multicast.is_ok() || send_broadcast.is_ok();

            if let Ok(mut guard) = runtime.lock() {
                guard.last_broadcast_at = Some(beacon.sent_at);
                if sent {
                    guard.tx_count += 1;
                }

                if let Err(err) = send_multicast {
                    guard.last_error = Some(format!("multicast send error: {err}"));
                }
                if let Err(err) = send_broadcast {
                    guard.last_error = Some(format!("broadcast send error: {err}"));
                }
            }
        }
        Err(err) => {
            let message = format!("serialize beacon failed: {err}");
            set_last_error(runtime, message);
        }
    }
}

pub(super) fn spawn_discovery_worker(
    identity: DeviceIdentity,
    runtime: Arc<Mutex<DiscoveryRuntime>>,
) {
    let builder = thread::Builder::new().name("fini-device-discovery".to_string());
    let runtime_worker = runtime.clone();

    let spawn_result = builder.spawn(move || {
        let runtime = runtime_worker;
        let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
            Ok(sock) => sock,
            Err(err) => {
                let message = format!("bind discovery socket failed: {err}");
                eprintln!("[device-sync] {message}");
                set_last_error(&runtime, message);
                return;
            }
        };

        let _ = socket.set_broadcast(true);
        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED);

        if let Ok(mut guard) = runtime.lock() {
            guard.worker_started = true;
        }

        let mut buffer = [0_u8; 2048];
        let discovery_interval = Duration::from_millis(DISCOVERY_INTERVAL_MS);
        let heartbeat_interval = Duration::from_millis(HEARTBEAT_INTERVAL_MS);
        let discovery_ttl = Duration::from_secs(DISCOVERY_TTL_SECS);
        let mut next_add_broadcast_at = Instant::now();
        let mut next_heartbeat_at = Instant::now();

        loop {
            let add_mode_enabled = runtime
                .lock()
                .map(|guard| guard.add_mode_enabled)
                .unwrap_or(false);
            let now = Instant::now();

            if now >= next_heartbeat_at {
                broadcast_beacon(&socket, &runtime, &identity, "heartbeat");
                next_heartbeat_at = now + heartbeat_interval;
            }

            if add_mode_enabled {
                if now >= next_add_broadcast_at {
                    broadcast_beacon(&socket, &runtime, &identity, "add");
                    next_add_broadcast_at = now + discovery_interval;
                }
            } else {
                next_add_broadcast_at = now;
            }

            match socket.recv_from(&mut buffer) {
                Ok((size, addr)) => {
                    if let Ok(beacon) = serde_json::from_slice::<DiscoveryBeacon>(&buffer[..size]) {
                        if beacon.protocol == DISCOVERY_PROTOCOL
                            && beacon.device_id != identity.device_id
                        {
                            if let Ok(mut guard) = runtime.lock() {
                                guard.rx_count += 1;

                                upsert_seen_peer(&mut guard.presence, &beacon, addr.ip());

                                if guard.add_mode_enabled && beacon.mode == "add" {
                                    let is_new =
                                        !guard.discovered.contains_key(beacon.device_id.as_str());

                                    upsert_seen_peer(&mut guard.discovered, &beacon, addr.ip());

                                    if is_new {
                                        eprintln!(
                                            "[device-sync] discovered peer {} ({}) at {}",
                                            beacon.hostname, beacon.device_id, addr
                                        );
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    if let Ok(pair_request) =
                        serde_json::from_slice::<PairRequestPayload>(&buffer[..size])
                    {
                        if pair_request.protocol == DISCOVERY_PROTOCOL
                            && pair_request.kind == "pair_request"
                            && pair_request.to_device_id == identity.device_id
                        {
                            if let Ok(mut guard) = runtime.lock() {
                                guard.rx_count += 1;
                                if guard.add_mode_enabled {
                                    let is_new = !guard
                                        .incoming_requests
                                        .contains_key(pair_request.request_id.as_str());

                                    guard.incoming_requests.insert(
                                        pair_request.request_id.clone(),
                                        build_incoming_pair_request(
                                            &pair_request,
                                            addr.ip().to_string(),
                                        ),
                                    );

                                    if is_new {
                                        eprintln!(
                                            "[device-sync] incoming pair request {} from {} ({})",
                                            pair_request.request_id,
                                            pair_request.from_hostname,
                                            pair_request.from_device_id
                                        );
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    if let Ok(pair_accept) =
                        serde_json::from_slice::<PairAcceptPayload>(&buffer[..size])
                    {
                        if pair_accept.protocol == DISCOVERY_PROTOCOL
                            && pair_accept.kind == "pair_accept"
                            && pair_accept.to_device_id == identity.device_id
                        {
                            if let Ok(mut guard) = runtime.lock() {
                                guard.rx_count += 1;
                                guard.outgoing_code_updates.insert(
                                    pair_accept.request_id.clone(),
                                    PairCodeUpdate {
                                        request_id: pair_accept.request_id.clone(),
                                        code: pair_accept.code.clone(),
                                        accepted_at: pair_accept.accepted_at.clone(),
                                    },
                                );

                                eprintln!(
                                    "[device-sync] pair request {} accepted by {} ({})",
                                    pair_accept.request_id, pair_accept.from_device_id, addr
                                );
                            }
                        }
                        continue;
                    }

                    if let Ok(pair_complete) =
                        serde_json::from_slice::<PairCompletePayload>(&buffer[..size])
                    {
                        if pair_complete.protocol == DISCOVERY_PROTOCOL
                            && pair_complete.kind == "pair_complete"
                            && pair_complete.to_device_id == identity.device_id
                        {
                            if let Ok(mut guard) = runtime.lock() {
                                guard.rx_count += 1;
                                guard.outgoing_pair_completions.insert(
                                    pair_complete.request_id.clone(),
                                    PairCompletionUpdate {
                                        request_id: pair_complete.request_id.clone(),
                                        from_device_id: pair_complete.from_device_id.clone(),
                                        from_hostname: pair_complete.from_hostname.clone(),
                                        paired_at: pair_complete.paired_at.clone(),
                                    },
                                );

                                eprintln!(
                                    "[device-sync] pair request {} completed by {} ({})",
                                    pair_complete.request_id, pair_complete.from_device_id, addr
                                );
                            }
                        }
                        continue;
                    }
                }
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut => {}
                Err(err) => {
                    let message = format!("discovery recv error: {err}");
                    set_last_error(&runtime, message);
                }
            }

            if let Ok(mut guard) = runtime.lock() {
                guard
                    .discovered
                    .retain(|_, peer| peer.last_seen_mono.elapsed() <= discovery_ttl);
                prune_expired_incoming_requests(&mut guard);

                if !guard.add_mode_enabled {
                    guard.discovered.clear();
                    guard.incoming_requests.clear();
                    guard.outgoing_code_updates.clear();
                    guard.outgoing_pair_completions.clear();
                }
            }
        }
    });

    if let Err(err) = spawn_result {
        let message = format!("spawn discovery worker failed: {err}");
        eprintln!("[device-sync] {message}");
        set_last_error(&runtime, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use std::path::PathBuf;

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
            to_device_id: "device-b".to_string(),
            created_at: "2000-01-01T00:00:00Z".to_string(),
            expires_at: expires_at.to_string(),
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
            },
            from_addr: "127.0.0.1".to_string(),
        }
    }

    fn sample_beacon(mode: &str, device_id: &str, hostname: &str) -> DiscoveryBeacon {
        DiscoveryBeacon {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            mode: mode.to_string(),
            device_id: device_id.to_string(),
            hostname: hostname.to_string(),
            sent_at: utc_now(),
            ws_port: Some(SPACE_SYNC_WS_PORT),
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
    fn upsert_seen_peer_overwrites_existing_peer_with_latest_identity() {
        let mut peers = HashMap::new();
        let first = sample_beacon("heartbeat", "device-a", "alpha");
        let second = sample_beacon("heartbeat", "device-a", "alpha-renamed");

        upsert_seen_peer(
            &mut peers,
            &first,
            "192.168.1.10".parse().expect("parse ip"),
        );
        upsert_seen_peer(
            &mut peers,
            &second,
            "192.168.1.11".parse().expect("parse ip"),
        );

        let peer = peers.get("device-a").expect("peer should exist");
        assert_eq!(peer.hostname, "alpha-renamed");
        assert_eq!(peer.addr, "192.168.1.11");
        assert!(
            parse_utc_timestamp(&peer.last_seen_at).is_some(),
            "last_seen_at should remain valid UTC"
        );
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

        prune_expired_incoming_requests(&mut runtime);

        assert!(runtime.incoming_requests.contains_key("active"));
        assert!(!runtime.incoming_requests.contains_key("expired"));
        assert!(!runtime.incoming_requests.contains_key("invalid"));
    }

    #[test]
    fn incoming_pair_request_uses_receiver_local_ttl() {
        let payload = sample_pair_request_payload("1999-01-01T00:00:00Z");
        let before = Utc::now();
        let stored = build_incoming_pair_request(&payload, "192.168.1.50".to_string());
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
    fn load_or_create_identity_creates_and_persists_new_identity() {
        let dir = unique_temp_dir("identity-create");
        let identity = load_or_create_identity(&dir);

        assert!(!identity.device_id.trim().is_empty());
        assert!(!identity.hostname.trim().is_empty());

        let saved_raw = std::fs::read_to_string(dir.join("device_identity.json"))
            .expect("identity file should exist");
        let saved: DeviceIdentity =
            serde_json::from_str(&saved_raw).expect("saved identity should parse");

        assert_eq!(saved.device_id, identity.device_id);
        assert_eq!(saved.hostname, identity.hostname);
    }

    #[test]
    fn load_or_create_identity_reuses_existing_valid_identity() {
        let dir = unique_temp_dir("identity-reuse");
        let existing = DeviceIdentity {
            device_id: "existing-device-id".to_string(),
            hostname: "existing-host".to_string(),
        };

        let payload = serde_json::to_string_pretty(&existing).expect("serialize fixture identity");
        std::fs::write(dir.join("device_identity.json"), payload)
            .expect("failed to write fixture identity file");

        let loaded = load_or_create_identity(&dir);
        assert_eq!(loaded.device_id, existing.device_id);
        assert_eq!(loaded.hostname, existing.hostname);
    }

    #[test]
    fn load_or_create_identity_replaces_invalid_identity_file() {
        let dir = unique_temp_dir("identity-invalid");
        std::fs::write(dir.join("device_identity.json"), "not-json")
            .expect("failed to seed invalid identity file");

        let loaded = load_or_create_identity(&dir);

        assert!(!loaded.device_id.trim().is_empty());
        assert!(!loaded.hostname.trim().is_empty());

        let saved_raw = std::fs::read_to_string(dir.join("device_identity.json"))
            .expect("identity file should be rewritten");
        let saved: DeviceIdentity =
            serde_json::from_str(&saved_raw).expect("rewritten identity should parse");

        assert_eq!(saved.device_id, loaded.device_id);
        assert_eq!(saved.hostname, loaded.hostname);
    }
}
