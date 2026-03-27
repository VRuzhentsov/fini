use chrono::Utc;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;
use tauri::State;

use super::{DISCOVERY_PORT, DISCOVERY_PROTOCOL, DISCOVERY_TTL_SECS, PAIR_REQUEST_TTL_SECS};
use crate::services::device_sync::runtime::{
    generate_passcode, prune_expired_incoming_requests, utc_now,
};
use crate::services::device_sync::types::{
    DeviceIdentity, DevicePairRequestAckInput, DevicePairRequestInput, DeviceSyncDebugStatus,
    DiscoveredDevice, IncomingPairRequest, PairAcceptPayload, PairCodeUpdate, PairCompletePayload,
    PairCompletionUpdate, PairRequestPayload,
};
use crate::services::device_sync::DeviceSyncState;

#[tauri::command]
pub fn device_get_identity(state: State<DeviceSyncState>) -> Result<DeviceIdentity, String> {
    Ok(state.identity.clone())
}

#[tauri::command]
pub fn device_enter_add_mode(state: State<DeviceSyncState>) -> Result<(), String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;
    guard.add_mode_enabled = true;
    guard.last_error = None;
    eprintln!(
        "[device-sync] add mode enabled for {} ({})",
        state.identity.hostname, state.identity.device_id
    );
    Ok(())
}

#[tauri::command]
pub fn device_leave_add_mode(state: State<DeviceSyncState>) -> Result<(), String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;
    guard.add_mode_enabled = false;
    guard.discovered.clear();
    guard.incoming_requests.clear();
    guard.outgoing_code_updates.clear();
    guard.outgoing_pair_completions.clear();
    eprintln!(
        "[device-sync] add mode disabled for {} ({})",
        state.identity.hostname, state.identity.device_id
    );
    Ok(())
}

#[tauri::command]
pub fn device_send_pair_request(
    state: State<DeviceSyncState>,
    input: DevicePairRequestInput,
) -> Result<(), String> {
    let target_ip: IpAddr = input
        .to_addr
        .parse()
        .map_err(|err| format!("invalid peer addr '{}': {err}", input.to_addr))?;

    let created_at = utc_now();
    let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let payload = PairRequestPayload {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        kind: "pair_request".to_string(),
        request_id: input.request_id,
        from_device_id: state.identity.device_id.clone(),
        from_hostname: state.identity.hostname.clone(),
        to_device_id: input.to_device_id,
        created_at,
        expires_at,
    };

    let bytes =
        serde_json::to_vec(&payload).map_err(|err| format!("serialize pair request: {err}"))?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("bind ephemeral send socket failed: {err}"))?;

    let target = SocketAddr::new(target_ip, DISCOVERY_PORT);
    socket
        .send_to(&bytes, target)
        .map_err(|err| format!("send pair request failed: {err}"))?;

    if let Ok(mut guard) = state.runtime.lock() {
        guard.tx_count += 1;
    }

    eprintln!(
        "[device-sync] pair request {} sent to {} ({})",
        payload.request_id, payload.to_device_id, target
    );

    Ok(())
}

#[tauri::command]
pub fn device_pair_incoming_requests(
    state: State<DeviceSyncState>,
) -> Result<Vec<IncomingPairRequest>, String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    prune_expired_incoming_requests(&mut guard);

    let mut requests: Vec<IncomingPairRequest> = guard
        .incoming_requests
        .values()
        .map(|item| item.request.clone())
        .collect();
    requests.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });

    Ok(requests)
}

#[tauri::command]
pub fn device_pair_outgoing_updates(
    state: State<DeviceSyncState>,
) -> Result<Vec<PairCodeUpdate>, String> {
    let guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    let mut updates: Vec<PairCodeUpdate> = guard.outgoing_code_updates.values().cloned().collect();
    updates.sort_by(|a, b| {
        b.accepted_at
            .cmp(&a.accepted_at)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });

    Ok(updates)
}

#[tauri::command]
pub fn device_pair_outgoing_completions(
    state: State<DeviceSyncState>,
) -> Result<Vec<PairCompletionUpdate>, String> {
    let guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    let mut updates: Vec<PairCompletionUpdate> =
        guard.outgoing_pair_completions.values().cloned().collect();
    updates.sort_by(|a, b| {
        b.paired_at
            .cmp(&a.paired_at)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });

    Ok(updates)
}

#[tauri::command]
pub fn device_pair_accept_request(
    state: State<DeviceSyncState>,
    input: DevicePairRequestAckInput,
) -> Result<PairCodeUpdate, String> {
    let (to_device_id, to_addr) = {
        let mut guard = state
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;

        prune_expired_incoming_requests(&mut guard);

        let Some(stored) = guard.incoming_requests.get(&input.request_id) else {
            return Err("incoming request not found".to_string());
        };

        (
            stored.request.from_device_id.clone(),
            stored.from_addr.clone(),
        )
    };

    let target_ip: IpAddr = to_addr
        .parse()
        .map_err(|err| format!("invalid sender addr '{}': {err}", to_addr))?;

    let update = PairCodeUpdate {
        request_id: input.request_id,
        code: generate_passcode(),
        accepted_at: utc_now(),
    };

    let payload = PairAcceptPayload {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        kind: "pair_accept".to_string(),
        request_id: update.request_id.clone(),
        code: update.code.clone(),
        from_device_id: state.identity.device_id.clone(),
        to_device_id: to_device_id.clone(),
        accepted_at: update.accepted_at.clone(),
    };

    let bytes =
        serde_json::to_vec(&payload).map_err(|err| format!("serialize pair accept: {err}"))?;
    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("bind pair accept socket failed: {err}"))?;
    let target = SocketAddr::new(target_ip, DISCOVERY_PORT);
    let mut sent_count: u64 = 0;
    let mut last_send_error: Option<String> = None;

    for _ in 0..3 {
        match socket.send_to(&bytes, target) {
            Ok(_) => {
                sent_count += 1;
            }
            Err(err) => {
                last_send_error = Some(err.to_string());
            }
        }
    }

    if sent_count == 0 {
        let message = last_send_error.unwrap_or_else(|| "unknown send error".to_string());
        return Err(format!("send pair accept failed: {message}"));
    }

    if let Ok(mut guard) = state.runtime.lock() {
        guard.tx_count += sent_count;
    }

    eprintln!(
        "[device-sync] accepted request {} for {} with code {} (sent {}x)",
        update.request_id, to_device_id, update.code, sent_count
    );

    Ok(update)
}

#[tauri::command]
pub fn device_pair_complete_request(
    state: State<DeviceSyncState>,
    input: DevicePairRequestAckInput,
) -> Result<(), String> {
    let (to_device_id, to_addr) = {
        let mut guard = state
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;

        prune_expired_incoming_requests(&mut guard);

        let Some(stored) = guard.incoming_requests.get(&input.request_id) else {
            return Err("incoming request not found".to_string());
        };

        (
            stored.request.from_device_id.clone(),
            stored.from_addr.clone(),
        )
    };

    let target_ip: IpAddr = to_addr
        .parse()
        .map_err(|err| format!("invalid sender addr '{}': {err}", to_addr))?;

    let payload = PairCompletePayload {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        kind: "pair_complete".to_string(),
        request_id: input.request_id.clone(),
        from_device_id: state.identity.device_id.clone(),
        from_hostname: state.identity.hostname.clone(),
        to_device_id: to_device_id.clone(),
        paired_at: utc_now(),
    };

    let bytes =
        serde_json::to_vec(&payload).map_err(|err| format!("serialize pair complete: {err}"))?;
    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("bind pair complete socket failed: {err}"))?;
    let target = SocketAddr::new(target_ip, DISCOVERY_PORT);

    let mut sent_count: u64 = 0;
    let mut last_send_error: Option<String> = None;
    for _ in 0..3 {
        match socket.send_to(&bytes, target) {
            Ok(_) => {
                sent_count += 1;
            }
            Err(err) => {
                last_send_error = Some(err.to_string());
            }
        }
    }

    if sent_count == 0 {
        let message = last_send_error.unwrap_or_else(|| "unknown send error".to_string());
        return Err(format!("send pair complete failed: {message}"));
    }

    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;
    guard.tx_count += sent_count;
    guard.incoming_requests.remove(&input.request_id);

    eprintln!(
        "[device-sync] completed request {} for {} (sent {}x)",
        input.request_id, to_device_id, sent_count
    );

    Ok(())
}

#[tauri::command]
pub fn device_pair_acknowledge_request(
    state: State<DeviceSyncState>,
    input: DevicePairRequestAckInput,
) -> Result<(), String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    guard.incoming_requests.remove(&input.request_id);
    Ok(())
}

#[tauri::command]
pub fn device_discovery_snapshot(
    state: State<DeviceSyncState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    let ttl = Duration::from_secs(DISCOVERY_TTL_SECS);
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    guard
        .discovered
        .retain(|_, peer| peer.last_seen_mono.elapsed() <= ttl);

    let mut items: Vec<DiscoveredDevice> = guard
        .discovered
        .iter()
        .map(|(device_id, peer)| DiscoveredDevice {
            device_id: device_id.clone(),
            hostname: peer.hostname.clone(),
            addr: peer.addr.clone(),
            last_seen_at: peer.last_seen_at.clone(),
        })
        .collect();

    items.sort_by(|a, b| {
        b.last_seen_at
            .cmp(&a.last_seen_at)
            .then_with(|| a.device_id.cmp(&b.device_id))
    });

    Ok(items)
}

#[tauri::command]
pub fn device_presence_snapshot(
    state: State<DeviceSyncState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    let guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    let mut items: Vec<DiscoveredDevice> = guard
        .presence
        .iter()
        .map(|(device_id, peer)| DiscoveredDevice {
            device_id: device_id.clone(),
            hostname: peer.hostname.clone(),
            addr: peer.addr.clone(),
            last_seen_at: peer.last_seen_at.clone(),
        })
        .collect();

    items.sort_by(|a, b| {
        b.last_seen_at
            .cmp(&a.last_seen_at)
            .then_with(|| a.device_id.cmp(&b.device_id))
    });

    Ok(items)
}

#[tauri::command]
pub fn device_sync_debug_status(
    state: State<DeviceSyncState>,
) -> Result<DeviceSyncDebugStatus, String> {
    let guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    Ok(DeviceSyncDebugStatus {
        add_mode_enabled: guard.add_mode_enabled,
        worker_started: guard.worker_started,
        tx_count: guard.tx_count,
        rx_count: guard.rx_count,
        discovered_count: guard.discovered.len(),
        incoming_request_count: guard.incoming_requests.len(),
        outgoing_code_count: guard.outgoing_code_updates.len(),
        last_broadcast_at: guard.last_broadcast_at.clone(),
        last_error: guard.last_error.clone(),
        discovery_port: DISCOVERY_PORT,
    })
}
