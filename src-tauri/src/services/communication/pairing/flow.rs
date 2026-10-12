//! The pairing handshake (`specs/device-connect/README.md`): request,
//! accept, complete, acknowledge. Transport-neutral: each frame goes out
//! through the `PairingCarrier` for the channel the request came on.

use std::net::IpAddr;

use chrono::Utc;

use super::carrier::{NetworkCarrier, ReplyTo};
use super::commands::local_bluetooth_address;
use super::runtime::{generate_passcode, prune_expired_incoming_requests, utc_now};
#[cfg(any(feature = "ui-plane", test))]
use super::types::DevicePairRequestBluetoothInput;
use super::types::{
    DevicePairRequestAckInput, DevicePairRequestInput, PairAcceptPayload, PairCodeUpdate, PairCompletePayload,
    PairRequestPayload,
};
use super::{DeviceConnectionState, DISCOVERY_PROTOCOL, PAIR_REQUEST_TTL_SECS};
use crate::services::communication::sync::types::PeerFrame;

/// A `PairRequest` from this device to `to_device_id`.
fn request_payload(state: &DeviceConnectionState, request_id: String, to_device_id: String) -> PairRequestPayload {
    let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    PairRequestPayload {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        kind: "pair_request".to_string(),
        request_id,
        from_device_id: state.identity.device_id.clone(),
        from_endpoint_id: Some(state.identity.endpoint_id.clone()),
        from_hostname: state.identity.hostname.clone(),
        from_discovery_port: Some(state.discovery_port),
        from_ws_port: Some(state.space_sync_ws_port),
        to_device_id,
        created_at: utc_now(),
        expires_at,
    }
}

fn count_sent(state: &DeviceConnectionState) {
    if let Ok(mut guard) = state.runtime.lock() {
        guard.tx_count += 1;
    }
}

/// Sends a pair request over the Network channel.
pub(crate) fn send_request(state: &DeviceConnectionState, input: DevicePairRequestInput) -> Result<(), String> {
    let target_ip: IpAddr = input
        .to_addr
        .parse()
        .map_err(|err| format!("invalid peer addr '{}': {err}", input.to_addr))?;
    let payload = request_payload(state, input.request_id, input.to_device_id);

    let target_port = input.to_ws_port.unwrap_or(state.space_sync_ws_port);
    let peer_key = input
        .to_endpoint_id
        .or_else(|| state.presence_key(&payload.to_device_id))
        .ok_or_else(|| {
            "this device has not announced its key yet; try again in a moment, or pass its key".to_string()
        })?;
    NetworkCarrier.send(state, &peer_key, target_ip, target_port, PeerFrame::PairRequest(payload.clone()))?;
    count_sent(state);

    eprintln!(
        "[device-sync] pair request {} sent to {} ({}:{})",
        payload.request_id, payload.to_device_id, target_ip, target_port
    );
    Ok(())
}

/// BLE-first pairing (ADR 0002 Phase 3): sends the same `PairRequestPayload`
/// shape `send_request` does, just over Bluetooth -- `run_peer_gate`
/// handles the resulting `PeerFrame::PairRequest` identically regardless of
/// which channel carried it. `to_device_id` here comes from a prior
/// `scan_add_mode_candidates`/`DiscoveryHelloReply`, not typed in by the
/// user.
#[cfg(any(feature = "ui-plane", test))]
pub(crate) fn send_request_bluetooth(
    state: &DeviceConnectionState, input: DevicePairRequestBluetoothInput,
) -> Result<(), String> {
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (state, input);
        Err("Bluetooth is not available on this platform".to_string())
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let payload = request_payload(state, input.request_id, input.to_device_id);
        state.bluetooth_carrier.send_request(
            state,
            &payload.to_device_id,
            &input.to_bluetooth_address,
            PeerFrame::PairRequest(payload.clone()),
        )?;
        count_sent(state);

        eprintln!(
            "[device-sync] pair request {} sent to {} (bluetooth {})",
            payload.request_id, payload.to_device_id, input.to_bluetooth_address
        );
        Ok(())
    }
}

/// An incoming request still waiting for an answer, as far as answering it
/// needs: who sent it, where from, and over which channel.
struct Incoming {
    from_device_id: String,
    from_addr: String,
    from_ws_port: u16,
    from_key: Option<String>,
    via_bluetooth: bool,
}

impl Incoming {
    fn find(state: &DeviceConnectionState, request_id: &str) -> Result<Self, String> {
        let mut guard = state
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;
        prune_expired_incoming_requests(&mut guard);
        let stored = guard
            .incoming_requests
            .get(request_id)
            .ok_or_else(|| "incoming request not found".to_string())?;
        Ok(Self {
            from_device_id: stored.request.from_device_id.clone(),
            from_addr: stored.from_addr.clone(),
            from_ws_port: stored.from_ws_port.unwrap_or(state.space_sync_ws_port),
            from_key: stored.from_endpoint_id.clone(),
            via_bluetooth: stored.request.via_bluetooth,
        })
    }

    /// Sends `msg` back to the requester, over the channel the request
    /// came on.
    fn answer(&self, state: &DeviceConnectionState, request_id: &str, msg: PeerFrame, more: bool) -> Result<(), String> {
        let to = ReplyTo {
            request_id,
            addr: &self.from_addr,
            ws_port: self.from_ws_port,
            key: self.from_key.as_deref(),
        };
        state.pairing_carrier(self.via_bluetooth).reply(state, &to, msg, more)
    }
}

pub(crate) fn accept_request(
    state: &DeviceConnectionState, input: DevicePairRequestAckInput,
) -> Result<PairCodeUpdate, String> {
    let incoming = Incoming::find(state, &input.request_id)?;

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
        from_endpoint_id: Some(state.identity.endpoint_id.clone()),
        to_device_id: incoming.from_device_id.clone(),
        accepted_at: update.accepted_at.clone(),
    };

    // The complete follows on the same request.
    incoming.answer(state, &update.request_id, PeerFrame::PairAccept(payload), true)?;
    count_sent(state);

    eprintln!(
        "[device-sync] accepted request {} for {} with code {}",
        update.request_id, incoming.from_device_id, update.code
    );
    Ok(update)
}

pub(crate) fn complete_request(state: &DeviceConnectionState, input: DevicePairRequestAckInput) -> Result<(), String> {
    let incoming = Incoming::find(state, &input.request_id)?;

    let payload = PairCompletePayload {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        kind: "pair_complete".to_string(),
        request_id: input.request_id.clone(),
        from_device_id: state.identity.device_id.clone(),
        from_endpoint_id: Some(state.identity.endpoint_id.clone()),
        from_hostname: state.identity.hostname.clone(),
        to_device_id: incoming.from_device_id.clone(),
        paired_at: utc_now(),
        // Best-effort: shared regardless of which channel carries this
        // frame, so a network-carried completion can still hand the
        // requester a Bluetooth address to store (ADR 0002 Phase 3).
        // `local_bluetooth_address` is genuinely bounded internally now
        // (kills its subprocess on timeout), so blocking this synchronous
        // command on it can't hang the way it could before.
        bluetooth_address: tauri::async_runtime::block_on(local_bluetooth_address()),
        key_material: None,
    };

    incoming.answer(state, &input.request_id, PeerFrame::PairComplete(payload), false)?;

    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;
    guard.tx_count += 1;
    guard.incoming_requests.remove(&input.request_id);
    if let Some(key) = incoming.from_key {
        guard
            .pairing_keys
            .insert(input.request_id.clone(), (incoming.from_device_id.clone(), key));
    }

    eprintln!(
        "[device-sync] completed request {} for {}",
        input.request_id, incoming.from_device_id
    );
    Ok(())
}

pub(crate) fn acknowledge_request(state: &DeviceConnectionState, input: DevicePairRequestAckInput) -> Result<(), String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;
    guard.incoming_requests.remove(&input.request_id);
    Ok(())
}
