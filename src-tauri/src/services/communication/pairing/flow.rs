//! The pairing handshake (`docs/architecture/communication.md`, "Pairing"): request,
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

/// How long the identify step waits for the answer once the link is open.
#[cfg(all(any(feature = "ui-plane", test), any(target_os = "linux", target_os = "android")))]
const IDENTIFY_ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl DeviceConnectionState {
    /// Pair pressed on a device the Add Device list shows from its Bluetooth
    /// advertisement: one plain link to it (the scanner finds the newest
    /// address), this device's `IdentifyDevice`, and its `DeviceIdentified`
    /// answer -- device id, name and iroh key. The result is recorded, so
    /// the pair request that follows can connect over iroh.
    #[cfg(all(any(feature = "ui-plane", test), any(target_os = "linux", target_os = "android")))]
    pub(crate) async fn identify_heard_device(
        &self, fingerprint: [u8; crate::services::communication::channel::bluetooth::FINGERPRINT_LEN],
    ) -> Result<crate::services::communication::channel::bluetooth::AvailableDevice, String> {
        use crate::services::communication::channel::{recv_frame, send_frame};
        self.close_held_bluetooth_session_to(fingerprint).await;
        let scanner = &self.bluetooth.scanner;
        let started = std::time::Instant::now();
        let (mut link, address, _guards) = scanner.connect_heard_device(fingerprint).await.inspect_err(|err| {
            log::info!("[pairing][ble] identify failed after {:?}: {err}", started.elapsed());
        })?;
        let hello = PeerFrame::IdentifyDevice {
            device_id: self.identity.device_id.clone(),
            hostname: self.identity.hostname.clone(),
            endpoint_id: self.identity.endpoint_id.clone(),
        };
        send_frame(link.as_mut(), &hello).await?;
        let reply = tokio::time::timeout(IDENTIFY_ANSWER_TIMEOUT, recv_frame(link.as_mut())).await;
        log::info!("[pairing][ble] identify {address} took {:?}", started.elapsed());
        match reply {
            Ok(Some(Ok(PeerFrame::DeviceIdentified { device_id, hostname, endpoint_id }))) => {
                if device_id == self.identity.device_id {
                    return Err("that is this device".to_string());
                }
                let device = crate::services::communication::channel::bluetooth::AvailableDevice {
                    address,
                    device_id,
                    hostname,
                };
                scanner.available.record_identified(&device, endpoint_id);
                Ok(device)
            }
            _ => Err("the device did not answer -- make sure Add Device is open on it".to_string()),
        }
    }

    /// Closes a held Bluetooth session to the paired device that advertises
    /// `fingerprint`, and waits until it has released. While one is held,
    /// BlueZ refuses a second LE connection to the same device
    /// (`le-connection-abort-by-local`), so the identify step for adding it
    /// again could never connect. Pairing again replaces the pair anyway;
    /// the session connects again once Add Device closes.
    #[cfg(all(any(feature = "ui-plane", test), any(target_os = "linux", target_os = "android")))]
    async fn close_held_bluetooth_session_to(
        &self, fingerprint: [u8; crate::services::communication::channel::bluetooth::FINGERPRINT_LEN],
    ) {
        use super::ChannelKind;
        let paired: Vec<String> = tokio::task::block_in_place(|| {
            use diesel::prelude::*;
            crate::schema::paired_devices::table
                .select(crate::schema::paired_devices::peer_device_id)
                .load(&mut crate::services::db::open_db_at_path(&self.db_path))
                .unwrap_or_default()
        });
        let held = paired.into_iter().find(|peer| {
            crate::services::communication::channel::bluetooth::fingerprint_of(peer) == fingerprint
                && self.has_session_on(peer, ChannelKind::Bluetooth)
        });
        let Some(peer) = held else {
            return;
        };
        log::info!("[pairing][ble] closing the held Bluetooth session with {peer} to identify it again");
        self.close_session_on(&peer, ChannelKind::Bluetooth);
        let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while self.has_session_on(&peer, ChannelKind::Bluetooth) && tokio::time::Instant::now() < until {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        // Its BLE link too: while it is up, the identify connect to the
        // device's new address is aborted.
        let peer_key = tokio::task::block_in_place(|| {
            super::pinned_key(&mut crate::services::db::open_db_at_path(&self.db_path), &peer)
        })
        .and_then(|key| key.parse::<iroh::EndpointId>().ok());
        if let Some(peer_key) = peer_key {
            self.bluetooth
                .close_links_to(peer_key, crate::services::communication::channel::bluetooth::TIME_BEFORE_LINK_CLOSE)
                .await;
        }
    }

    /// A `PairStart` from this device to `to_device_id`.
    fn pair_request_payload(&self, request_id: String, to_device_id: String) -> PairRequestPayload {
        let expires_at = (Utc::now() + chrono::Duration::seconds(PAIR_REQUEST_TTL_SECS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        PairRequestPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_start".to_string(),
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
        NetworkCarrier.send(self, &peer_key, target_ip, target_port, PeerFrame::PairStart(payload.clone()))?;
        self.count_sent();

        eprintln!(
            "[device-sync] pair request {} sent to {} ({}:{})",
            payload.request_id, payload.to_device_id, target_ip, target_port
        );
        Ok(())
    }

    /// Sends the same `PairRequestPayload` shape `send_pair_request` does,
    /// just over Bluetooth -- `run_peer_gate` handles the resulting
    /// `PeerFrame::PairStart` identically regardless of which channel
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
                PeerFrame::PairStart(payload.clone()),
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
