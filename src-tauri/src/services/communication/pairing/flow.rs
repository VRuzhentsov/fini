//! The pairing handshake (`specs/device-connect/README.md`): request,
//! accept, complete, acknowledge. Transport-neutral: each frame goes out
//! through the `PairingCarrier` for the channel the request came on.
//!
//! Methods of `DeviceConnectionState`, because the pair requests, codes and
//! keys they work on are held by the state.

use std::net::IpAddr;

use chrono::Utc;

use super::carrier::{NetworkCarrier, ReplyTo};
use super::commands::local_bluetooth_address;
use super::runtime::{generate_passcode, utc_now};
#[cfg(any(feature = "ui-plane", test))]
use super::types::DevicePairRequestBluetoothInput;
use super::types::{
    DevicePairRequestAckInput, DevicePairRequestInput, PairAcceptPayload, PairCodeUpdate, PairCompletePayload,
    PairRequestPayload,
};
use super::{DeviceConnectionState, DISCOVERY_PROTOCOL, PAIR_REQUEST_TTL_SECS};
use crate::services::communication::sync::types::PeerFrame;

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

impl DeviceConnectionState {
    /// A `PairRequest` from this device to `to_device_id`.
    fn pair_request_payload(&self, request_id: String, to_device_id: String) -> PairRequestPayload {
        let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        PairRequestPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_request".to_string(),
            request_id,
            from_device_id: self.identity.device_id.clone(),
            from_endpoint_id: Some(self.identity.endpoint_id.clone()),
            from_hostname: self.identity.hostname.clone(),
            from_discovery_port: Some(self.discovery_port),
            from_ws_port: Some(self.space_sync_ws_port),
            to_device_id,
            created_at: utc_now(),
            expires_at,
        }
    }

    fn count_sent(&self) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.tx_count += 1;
        }
    }

    /// Sends a pair request over the Network channel.
    pub(crate) fn send_pair_request(&self, input: DevicePairRequestInput) -> Result<(), String> {
        let target_ip: IpAddr = input
            .to_addr
            .parse()
            .map_err(|err| format!("invalid peer addr '{}': {err}", input.to_addr))?;
        let payload = self.pair_request_payload(input.request_id, input.to_device_id);

        let target_port = input.to_ws_port.unwrap_or(self.space_sync_ws_port);
        let peer_key = input
            .to_endpoint_id
            .or_else(|| self.presence_key(&payload.to_device_id))
            .ok_or_else(|| {
                "this device has not announced its key yet; try again in a moment, or pass its key".to_string()
            })?;
        NetworkCarrier.send(self, &peer_key, target_ip, target_port, PeerFrame::PairRequest(payload.clone()))?;
        self.count_sent();

        eprintln!(
            "[device-sync] pair request {} sent to {} ({}:{})",
            payload.request_id, payload.to_device_id, target_ip, target_port
        );
        Ok(())
    }

    /// Sends the same `PairRequestPayload` shape `send_pair_request` does,
    /// just over Bluetooth -- `run_peer_gate` handles the resulting
    /// `PeerFrame::PairRequest` identically regardless of which channel
    /// carried it. `to_device_id` comes from an identify step, not typed in
    /// by the user.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn send_pair_request_bluetooth(&self, input: DevicePairRequestBluetoothInput) -> Result<(), String> {
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = input;
            Err("Bluetooth is not available on this platform".to_string())
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let payload = self.pair_request_payload(input.request_id, input.to_device_id);
            self.bluetooth_carrier.send_request(
                self,
                &payload.to_device_id,
                &input.to_bluetooth_address,
                PeerFrame::PairRequest(payload.clone()),
            )?;
            self.count_sent();

            eprintln!(
                "[device-sync] pair request {} sent to {} (bluetooth {})",
                payload.request_id, payload.to_device_id, input.to_bluetooth_address
            );
            Ok(())
        }
    }

    /// The incoming request `request_id`, if it is still there and not
    /// expired.
    fn incoming_pair_request(&self, request_id: &str) -> Result<Incoming, String> {
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;
        guard.prune_expired_incoming_requests();
        let stored = guard
            .incoming_requests
            .get(request_id)
            .ok_or_else(|| "incoming request not found".to_string())?;
        Ok(Incoming {
            from_device_id: stored.request.from_device_id.clone(),
            from_addr: stored.from_addr.clone(),
            from_ws_port: stored.from_ws_port.unwrap_or(self.space_sync_ws_port),
            from_key: stored.from_endpoint_id.clone(),
            via_bluetooth: stored.request.via_bluetooth,
        })
    }

    pub(crate) fn accept_pair_request(&self, input: DevicePairRequestAckInput) -> Result<PairCodeUpdate, String> {
        let incoming = self.incoming_pair_request(&input.request_id)?;

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
            from_device_id: self.identity.device_id.clone(),
            from_endpoint_id: Some(self.identity.endpoint_id.clone()),
            to_device_id: incoming.from_device_id.clone(),
            accepted_at: update.accepted_at.clone(),
        };

        // The complete follows on the same request.
        incoming.answer(self, &update.request_id, PeerFrame::PairAccept(payload), true)?;
        self.count_sent();

        eprintln!(
            "[device-sync] accepted request {} for {} with code {}",
            update.request_id, incoming.from_device_id, update.code
        );
        Ok(update)
    }

    pub(crate) fn complete_pair_request(&self, input: DevicePairRequestAckInput) -> Result<(), String> {
        let incoming = self.incoming_pair_request(&input.request_id)?;

        let payload = PairCompletePayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_complete".to_string(),
            request_id: input.request_id.clone(),
            from_device_id: self.identity.device_id.clone(),
            from_endpoint_id: Some(self.identity.endpoint_id.clone()),
            from_hostname: self.identity.hostname.clone(),
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

        incoming.answer(self, &input.request_id, PeerFrame::PairComplete(payload), false)?;

        let mut guard = self
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

    pub(crate) fn acknowledge_pair_request(&self, input: DevicePairRequestAckInput) -> Result<(), String> {
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;
        guard.incoming_requests.remove(&input.request_id);
        Ok(())
    }
}
