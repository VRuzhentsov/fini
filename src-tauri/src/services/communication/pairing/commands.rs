use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use futures_util::SinkExt;
use std::net::IpAddr;
use std::time::Duration;
#[cfg(any(feature = "ui-plane", test))]
use tauri::State;
use tokio_tungstenite::{connect_async, tungstenite::Message};


use super::channels;
use super::{DISCOVERY_PROTOCOL, DISCOVERY_TTL_SECS, PAIR_REQUEST_TTL_SECS};
use crate::models::{CreatePairedDeviceInput, PairedDevice};
use crate::schema::paired_devices;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
use crate::services::communication::pairing::runtime::{
    generate_passcode, prune_expired_incoming_requests, utc_now,
};
use crate::services::communication::pairing::types::{
    DeviceConnectionDebugStatus, DeviceIdentity,
    DevicePairRequestAckInput, DevicePairRequestBluetoothInput, DevicePairRequestInput,
    DiscoveredDevice, IncomingPairRequest, IncomingSpaceMappingUpdate, PairAcceptPayload,
    PairCodeUpdate, PairCompletePayload, PairCompletionUpdate, PairRequestPayload,
};
use crate::services::communication::pairing::DeviceConnectionState;
use crate::services::communication::pairing::{
    build_channel_statuses, ChannelLiveness, ChannelReason, ChannelStatus, ChannelStatusInputs,
};
use crate::services::communication::sync::types::PeerFrame;
use crate::services::communication::channel::ChannelKind;

fn ws_url(addr: IpAddr, port: u16) -> String {
    match addr {
        IpAddr::V4(_) => format!("ws://{addr}:{port}"),
        IpAddr::V6(_) => format!("ws://[{addr}]:{port}"),
    }
}

pub(crate) fn normalize_bluetooth_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_uppercase())
}

/// Runs `command` with a hard time limit that actually terminates it, not
/// merely bounds how long a caller waits: `kill_on_drop(true)` makes Tokio
/// send the kill signal (and reap the process via its own SIGCHLD-driven
/// reaper) the instant `tokio::time::timeout` below drops the still-running
/// future. A `spawn_blocking`-wrapped `std::process::Command::output()`
/// can't do this -- once the blocking call is in flight there is no way to
/// cancel it, so a permanently hung subprocess keeps its thread (and the
/// process itself) alive forever, and every periodic retry leaks another
/// one. `None` if the command fails to spawn, doesn't exit successfully, or
/// doesn't finish within `timeout`.
async fn run_command_with_timeout(mut command: tokio::process::Command, timeout: Duration) -> Option<String> {
    command
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().ok()?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// This device's own real Bluetooth adapter address, when the platform can
/// read it at all. `None` on Android: `BluetoothAdapter.getAddress()` has
/// returned a dummy value since Android 6.0 for every normal app (a
/// permanent platform privacy protection, not a bug to work around) — so
/// Android has nothing real to self-report over `PeerFrame::BluetoothAddressUpdate`
/// and instead is discovered by a peer's BLE scan (`channel::ble`'s
/// discovery path). Linux has no such restriction.
pub(crate) async fn local_bluetooth_address() -> Option<String> {
    // Test/CI escape hatch, mirroring `FINI_BLUETOOTH_PAIRED_ADDRESSES` above:
    // exercising the self-report send path deterministically can't depend on
    // whether the machine actually has a Bluetooth controller.
    if let Ok(value) = std::env::var("FINI_LOCAL_BLUETOOTH_ADDRESS") {
        return normalize_bluetooth_address(&value);
    }

    #[cfg(target_os = "linux")]
    {
        // `bluetoothctl show` (the local controller), not `info <address>`
        // (a remote peer's bond status, used by `bluetooth_address_is_os_paired`
        // above) — same Flatpak `flatpak-spawn --host` routing, same reason.
        let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
            let mut command = tokio::process::Command::new("flatpak-spawn");
            command.arg("--host").arg("bluetoothctl");
            command
        } else {
            tokio::process::Command::new("bluetoothctl")
        };
        command.arg("show");
        let stdout = run_command_with_timeout(command, Duration::from_secs(5)).await?;
        return stdout.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("Controller ")?;
            normalize_bluetooth_address(rest.split_whitespace().next()?)
        });
    }

    #[allow(unreachable_code)]
    None
}

/// Stores `address` as `peer_id`'s Bluetooth address. Returns `false`
/// always, or an error if the write itself failed.
///
/// The name is now wider than the behaviour: since ADR-0006 this enables
/// nothing. It kept the `maybe_enable` half and the `bool` return through
/// that change deliberately, because every caller already branches on the
/// result and renaming it is a mechanical change better made when the
/// pairing work (#169) touches these paths anyway.
///
/// Shared by both Phase 1 mechanisms of ADR 0002: `session::run_session`'s
/// inbound `BluetoothAddressUpdate` handler (self-report) and
/// `channel::ble`'s scan-and-auth discovery. Both already have remote
/// confirmation before calling this -- an authenticated `PeerFrame`
/// channel, or a live `AuthOk` from the discovered address -- so recording
/// what they learned is safe.
///
/// What it must not do is decide enablement. It used to, based on a live
/// OS-bond check, and the "confirmed unbonded" branch *disabled* Bluetooth
/// for the pair. With no bond consulted anywhere on the dial path that
/// check meant nothing, and on hardware it silently switched off a working
/// pair seconds before it would have connected. Enablement now comes only
/// from an explicit user action or a completed BLE pairing.
pub(crate) fn persist_bluetooth_address_and_maybe_enable(
    conn: &mut SqliteConnection, peer_id: &str, address: &str,
) -> Result<bool, String> {
    // ADR-0006: record the address, and leave the switch strictly alone.
    //
    // This used to branch on a live OS-bond check -- confirmed bonded
    // enabled the pair, confirmed unbonded *disabled* it, inconclusive did
    // nothing. All three are meaningless now that no bond is consulted
    // anywhere on the dial path, and the middle one was actively harmful:
    // it is what silently turned Bluetooth off for a working pair during
    // hardware verification. The desktop self-reported its address over the
    // network, the phone found no bond for it, and disabled the channel
    // that was about to connect. What reached the other side was
    // `auth rejected: bluetooth disabled for this pair`, with nothing
    // anywhere naming the cause.
    //
    // `note_address` writes only to a channel that exists and is on, which
    // is what keeps a self-report from undoing an explicit switch-off --
    // the job `bluetooth_disabled_by_user` used to hold a whole column for.
    // Switching the channel back on is what stores a fresh address again,
    // deliberately as its own distinct act.
    channels::note_address(conn, peer_id, ChannelKind::Bluetooth, address);
    Ok(false)
}

/// One-shot pre-auth pairing sender (`PairRequest`/`PairAccept`/`PairComplete`).
/// Independent of `channel::tcp_ws::TcpWsDataLink` (connect, send one frame,
/// close — no need for a full `DataLink`), but MUST encode via the same
/// `channel::codec::encode_frame` (envelope-wrapped) and the same `Message::Text`
/// framing `TcpWsDataLink` reads, or `run_peer_gate` silently fails to parse the
/// first frame.
fn send_pair_ws(addr: IpAddr, port: u16, msg: PeerFrame) -> Result<(), String> {
    tauri::async_runtime::block_on(async move {
        let url = ws_url(addr, port);
        let (mut ws, _) = connect_async(&url)
            .await
            .map_err(|err| format!("connect pair websocket {url} failed: {err}"))?;
        let bytes = crate::services::communication::channel::codec::encode_frame(&msg)
            .map_err(|err| format!("encode pair websocket message failed: {err}"))?;
        let text = String::from_utf8(bytes)
            .map_err(|err| format!("non-utf8 pair websocket message: {err}"))?;
        ws.send(Message::Text(text.into()))
            .await
            .map_err(|err| format!("send pair websocket message failed: {err}"))?;
        let _ = ws.close(None).await;
        Ok(())
    })
}

/// One-shot pre-auth pairing sender over Bluetooth — the BLE-first pairing
/// equivalent of `send_pair_ws` above (ADR 0002 Phase 3). No text-framing
/// dance needed here: `channel::send_frame` already handles encoding for
/// any `DataLink`, unlike the WebSocket path, which has to hand-roll a
/// `Message::Text` frame around the same codec.
#[cfg(any(target_os = "linux", target_os = "android"))]
/// A stale/unresponsive BLE candidate has no bound of its own here: `dial`
/// can hang trying to connect to a device that's since gone out of range,
/// and `send_frame` can hang on a stalled write. All three BLE pairing legs
/// (request/accept/complete) are synchronous Tauri commands that block on
/// this via `block_on`, so an unbounded hang here freezes the whole
/// command -- leaving pairing controls stuck disabled, and letting a retry
/// after the request TTL expires collide with the still-open earlier
/// attempt.
const SEND_PAIR_BLE_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(any(target_os = "linux", target_os = "android"))]
fn send_pair_ble(address: &str, msg: PeerFrame) -> Result<(), String> {
    tauri::async_runtime::block_on(async move {
        tokio::time::timeout(SEND_PAIR_BLE_TIMEOUT, async {
            let mut link = crate::services::communication::channel::ble::dial(address).await?;
            crate::services::communication::channel::send_frame(link.as_mut(), &msg).await
        })
        .await
        .map_err(|_| "bluetooth pairing send timed out".to_string())?
    })
}

pub fn device_connection_get_identity_impl(
    state: &DeviceConnectionState,
) -> Result<DeviceIdentity, String> {
    Ok(state.identity.clone())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_get_identity(
    state: State<DeviceConnectionState>,
) -> Result<DeviceIdentity, String> {
    device_connection_get_identity_impl(&state)
}

pub fn device_connection_enter_add_mode_impl(state: &DeviceConnectionState) -> Result<(), String> {
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
    // One toggle, both channels (ADR 0002 Phase 3): entering add-mode
    // makes this device discoverable over Bluetooth too, not just the
    // existing mDNS beacon.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::communication::channel::ble::set_add_mode(true);
    // Opening Add Device is a genuine user action, the right point to
    // prompt -- see `BluetoothPairing.requestPermissionsIfNeeded`'s doc
    // comment. Requested exactly once per add-mode entry, here, not from
    // `device_connection_discover_bluetooth_candidates`: that command is
    // invoked repeatedly by the frontend's self-rescheduling scan loop
    // (every ~2s for as long as this view stays open), and prompting
    // again on every retry after the user has already explicitly denied
    // it once would violate that same contract.
    #[cfg(target_os = "android")]
    crate::services::android_context::call_static_context_void(
        "com.fini.app.BluetoothPairing",
        "requestPermissionsIfNeeded",
    );
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_enter_add_mode(state: State<DeviceConnectionState>) -> Result<(), String> {
    device_connection_enter_add_mode_impl(&state)
}

pub fn device_connection_leave_add_mode_impl(state: &DeviceConnectionState) -> Result<(), String> {
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
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::communication::channel::ble::set_add_mode(false);
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_leave_add_mode(state: State<DeviceConnectionState>) -> Result<(), String> {
    device_connection_leave_add_mode_impl(&state)
}

pub fn device_connection_send_pair_request_impl(
    state: &DeviceConnectionState,
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
        from_discovery_port: Some(state.discovery_port),
        from_ws_port: Some(state.space_sync_ws_port),
        to_device_id: input.to_device_id,
        created_at,
        expires_at,
    };

    let target_port = input.to_ws_port.unwrap_or(state.space_sync_ws_port);
    send_pair_ws(
        target_ip,
        target_port,
        PeerFrame::PairRequest(payload.clone()),
    )?;

    if let Ok(mut guard) = state.runtime.lock() {
        guard.tx_count += 1;
    }

    eprintln!(
        "[device-sync] pair request {} sent to {} ({}:{})",
        payload.request_id, payload.to_device_id, target_ip, target_port
    );

    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_send_pair_request(
    state: State<DeviceConnectionState>,
    input: DevicePairRequestInput,
) -> Result<(), String> {
    device_connection_send_pair_request_impl(&state, input)
}

/// BLE-first pairing (ADR 0002 Phase 3): sends the same `PairRequestPayload`
/// shape `device_connection_send_pair_request_impl` does, just over a fresh
/// Bluetooth connection instead of a WebSocket one -- `run_peer_gate`
/// handles the resulting `PeerFrame::PairRequest` identically regardless of
/// which channel carried it, so nothing downstream of `send_pair_ble`
/// needs to know the difference. `to_device_id` here comes from a prior
/// `scan_add_mode_candidates`/`DiscoveryHelloReply`, not typed in by the
/// user.
pub fn device_connection_send_pair_request_bluetooth_impl(
    state: &DeviceConnectionState,
    input: DevicePairRequestBluetoothInput,
) -> Result<(), String> {
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (state, input);
        return Err("Bluetooth is not available on this platform".to_string());
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
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
            from_discovery_port: Some(state.discovery_port),
            from_ws_port: Some(state.space_sync_ws_port),
            to_device_id: input.to_device_id,
            created_at,
            expires_at,
        };

        send_pair_ble(&input.to_bluetooth_address, PeerFrame::PairRequest(payload.clone()))?;

        if let Ok(mut guard) = state.runtime.lock() {
            guard.tx_count += 1;
        }

        eprintln!(
            "[device-sync] pair request {} sent to {} (bluetooth {})",
            payload.request_id, payload.to_device_id, input.to_bluetooth_address
        );

        Ok(())
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_send_pair_request_bluetooth(
    state: State<DeviceConnectionState>,
    input: DevicePairRequestBluetoothInput,
) -> Result<(), String> {
    device_connection_send_pair_request_bluetooth_impl(&state, input)
}

/// Phase 3's discovery scan, exposed to `AddDeviceView.vue`: scans for
/// add-mode-flagged BLE candidates for `duration_ms` and maps them into the
/// same `DiscoveredDevice` shape mDNS-sourced candidates use, tagged
/// `channel: Bluetooth`, for the unified candidate list.
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub async fn device_connection_discover_bluetooth_candidates(
    state: State<'_, DeviceConnectionState>, duration_ms: u64,
) -> Result<Vec<DiscoveredDevice>, String> {
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = duration_ms;
        let _ = state.identity.device_id.as_str();
        return Err("Bluetooth is not available on this platform".to_string());
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // Only *rechecks* permission here, doesn't re-prompt:
        // `device_connection_enter_add_mode_impl` already requested it once
        // for this add-mode session. This command is invoked repeatedly by
        // the frontend's self-rescheduling scan loop (every ~2s for as
        // long as Add Device stays open), and re-prompting on every retry
        // after an explicit denial would violate
        // `BluetoothPairing.requestPermissionsIfNeeded`'s "tied to a
        // genuine user action" contract.
        #[cfg(target_os = "android")]
        {
            if !crate::services::android_context::call_static_context_to_bool(
                "com.fini.app.BluetoothPairing",
                "hasPermissions",
            ) {
                return Err(
                    "Bluetooth permission required -- grant it in the dialog, then try again"
                        .to_string(),
                );
            }
        }

        let my_device_id = state.identity.device_id.clone();
        let candidates = crate::services::communication::channel::ble::scan_add_mode_candidates(
            &my_device_id,
            std::time::Duration::from_millis(duration_ms),
        )
        .await?;
        let now = utc_now();
        Ok(candidates
            .into_iter()
            .map(|candidate| DiscoveredDevice {
                device_id: candidate.device_id,
                hostname: candidate.hostname,
                addr: candidate.address,
                discovery_port: 0,
                ws_port: None,
                last_seen_at: now.clone(),
                channel_kind: crate::services::communication::pairing::channel_status::ChannelKind::Bluetooth,
            })
            .collect())
    }
}

pub fn device_connection_pair_incoming_requests_impl(
    state: &DeviceConnectionState,
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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_pair_incoming_requests(
    state: State<DeviceConnectionState>,
) -> Result<Vec<IncomingPairRequest>, String> {
    device_connection_pair_incoming_requests_impl(&state)
}

pub fn device_connection_pair_outgoing_updates_impl(
    state: &DeviceConnectionState,
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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_pair_outgoing_updates(
    state: State<DeviceConnectionState>,
) -> Result<Vec<PairCodeUpdate>, String> {
    device_connection_pair_outgoing_updates_impl(&state)
}

pub fn device_connection_pair_outgoing_completions_impl(
    state: &DeviceConnectionState,
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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_pair_outgoing_completions(
    state: State<DeviceConnectionState>,
) -> Result<Vec<PairCompletionUpdate>, String> {
    device_connection_pair_outgoing_completions_impl(&state)
}

pub fn device_connection_pair_accept_request_impl(
    state: &DeviceConnectionState,
    input: DevicePairRequestAckInput,
) -> Result<PairCodeUpdate, String> {
    let (to_device_id, to_addr, to_ws_port, via_bluetooth) = {
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
            stored.from_ws_port.unwrap_or(state.space_sync_ws_port),
            stored.request.via_bluetooth,
        )
    };

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

    if via_bluetooth {
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        return Err("Bluetooth is not available on this platform".to_string());
        #[cfg(any(target_os = "linux", target_os = "android"))]
        send_pair_ble(&to_addr, PeerFrame::PairAccept(payload))?;
    } else {
        let target_ip: IpAddr = to_addr
            .parse()
            .map_err(|err| format!("invalid sender addr '{}': {err}", to_addr))?;
        send_pair_ws(target_ip, to_ws_port, PeerFrame::PairAccept(payload))?;
    }

    if let Ok(mut guard) = state.runtime.lock() {
        guard.tx_count += 1;
    }

    eprintln!(
        "[device-sync] accepted request {} for {} with code {}",
        update.request_id, to_device_id, update.code
    );

    Ok(update)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_pair_accept_request(
    state: State<DeviceConnectionState>,
    input: DevicePairRequestAckInput,
) -> Result<PairCodeUpdate, String> {
    device_connection_pair_accept_request_impl(&state, input)
}

pub fn device_connection_pair_complete_request_impl(
    state: &DeviceConnectionState,
    input: DevicePairRequestAckInput,
) -> Result<(), String> {
    let (to_device_id, to_addr, to_ws_port, via_bluetooth) = {
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
            stored.from_ws_port.unwrap_or(state.space_sync_ws_port),
            stored.request.via_bluetooth,
        )
    };

    let payload = PairCompletePayload {
        protocol: DISCOVERY_PROTOCOL.to_string(),
        kind: "pair_complete".to_string(),
        request_id: input.request_id.clone(),
        from_device_id: state.identity.device_id.clone(),
        from_hostname: state.identity.hostname.clone(),
        to_device_id: to_device_id.clone(),
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

    if via_bluetooth {
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        return Err("Bluetooth is not available on this platform".to_string());
        #[cfg(any(target_os = "linux", target_os = "android"))]
        send_pair_ble(&to_addr, PeerFrame::PairComplete(payload))?;
    } else {
        let target_ip: IpAddr = to_addr
            .parse()
            .map_err(|err| format!("invalid sender addr '{}': {err}", to_addr))?;
        send_pair_ws(target_ip, to_ws_port, PeerFrame::PairComplete(payload))?;
    }

    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;
    guard.tx_count += 1;
    guard.incoming_requests.remove(&input.request_id);

    eprintln!(
        "[device-sync] completed request {} for {}",
        input.request_id, to_device_id
    );

    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_pair_complete_request(
    state: State<DeviceConnectionState>,
    input: DevicePairRequestAckInput,
) -> Result<(), String> {
    device_connection_pair_complete_request_impl(&state, input)
}

pub fn device_connection_pair_acknowledge_request_impl(
    state: &DeviceConnectionState,
    input: DevicePairRequestAckInput,
) -> Result<(), String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    guard.incoming_requests.remove(&input.request_id);
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_pair_acknowledge_request(
    state: State<DeviceConnectionState>,
    input: DevicePairRequestAckInput,
) -> Result<(), String> {
    device_connection_pair_acknowledge_request_impl(&state, input)
}

pub fn device_connection_discovery_snapshot_impl(
    state: &DeviceConnectionState,
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
            discovery_port: peer.discovery_port,
            ws_port: peer.ws_port,
            last_seen_at: peer.last_seen_at.clone(),
            channel_kind: Default::default(),
        })
        .collect();

    items.sort_by(|a, b| {
        b.last_seen_at
            .cmp(&a.last_seen_at)
            .then_with(|| a.device_id.cmp(&b.device_id))
    });

    Ok(items)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_discovery_snapshot(
    state: State<DeviceConnectionState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    device_connection_discovery_snapshot_impl(&state)
}

pub fn device_connection_presence_snapshot_impl(
    state: &DeviceConnectionState,
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
            discovery_port: peer.discovery_port,
            ws_port: peer.ws_port,
            last_seen_at: peer.last_seen_at.clone(),
            channel_kind: Default::default(),
        })
        .collect();

    items.sort_by(|a, b| {
        b.last_seen_at
            .cmp(&a.last_seen_at)
            .then_with(|| a.device_id.cmp(&b.device_id))
    });

    Ok(items)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_presence_snapshot(
    state: State<DeviceConnectionState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    device_connection_presence_snapshot_impl(&state)
}

pub fn device_connection_debug_status_impl(
    state: &DeviceConnectionState,
) -> Result<DeviceConnectionDebugStatus, String> {
    let guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    Ok(DeviceConnectionDebugStatus {
        add_mode_enabled: guard.add_mode_enabled,
        worker_started: guard.worker_started,
        tx_count: guard.tx_count,
        rx_count: guard.rx_count,
        discovered_count: guard.discovered.len(),
        peer_session_count: guard.peer_sessions.len(),
        incoming_request_count: guard.incoming_requests.len(),
        incoming_space_mapping_update_count: guard.incoming_space_mapping_updates.len(),
        outgoing_code_count: guard.outgoing_code_updates.len(),
        last_broadcast_at: guard.last_broadcast_at.clone(),
        last_error: guard.last_error.clone(),
        discovery_port: state.discovery_port,
        discovery_provider: "mdns-sd".to_string(),
    })
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_debug_status(
    state: State<DeviceConnectionState>,
) -> Result<DeviceConnectionDebugStatus, String> {
    device_connection_debug_status_impl(&state)
}

pub fn device_connection_consume_space_mapping_updates_impl(
    state: &DeviceConnectionState,
) -> Result<Vec<IncomingSpaceMappingUpdate>, String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "device sync runtime lock poisoned".to_string())?;

    let mut updates: Vec<IncomingSpaceMappingUpdate> = guard
        .incoming_space_mapping_updates
        .drain()
        .map(|(_, v)| v)
        .collect();
    updates.sort_by(|a, b| {
        b.sent_at
            .cmp(&a.sent_at)
            .then_with(|| a.from_device_id.cmp(&b.from_device_id))
    });
    Ok(updates)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_consume_space_mapping_updates(
    state: State<DeviceConnectionState>,
) -> Result<Vec<IncomingSpaceMappingUpdate>, String> {
    device_connection_consume_space_mapping_updates_impl(&state)
}

// ── Paired device CRUD (SQLite) ──────────────────────────────────────────────

pub fn device_connection_get_paired_devices_impl(
    conn: &mut SqliteConnection,
) -> Result<Vec<PairedDevice>, String> {
    paired_devices::table
        .select(PairedDevice::as_select())
        .order(paired_devices::paired_at.desc())
        .load(conn)
        .map_err(|e| e.to_string())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_get_paired_devices(
    db: State<AppDbConnection>,
) -> Result<Vec<PairedDevice>, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_get_paired_devices_impl(&mut conn)
}

pub fn device_connection_save_paired_device_impl(
    conn: &mut SqliteConnection,
    peer_device_id: String,
    display_name: String,
    bluetooth_address: Option<String>,
    via_bluetooth: bool,
    // Unused since ADR-0006 removed the `request_os_bond` call this fed: a
    // completed BLE pairing no longer needs the OS to bond anything, so
    // there is nothing here that wants a second DB handle. Kept in the
    // signature rather than removed because the callers, the Tauri command
    // and its tests all pass it, and churning that is not this change's
    // job -- the BLE pairing work (#169) will settle it either way.
    _db_path: std::path::PathBuf,
) -> Result<PairedDevice, String> {
    // A P2 review finding: `ble::dial_exhausted`/`dial_backoff_until`/
    // `accepting_side_unconnected_since` are process-global, keyed only by
    // `peer_device_id` -- a peer that exhausted, got unpaired, and was
    // paired again under the same id (without an app restart) used to come
    // back already exhausted, with `spawn_dial_loop` skipping it and its row
    // reporting exhausted immediately instead of getting a fresh
    // `AUTO_RETRY_WINDOW`. A no-op the vast majority of the time (a peer
    // that was never exhausted has nothing to clear); see
    // `device_connection_unpair_impl` for the unpair-side half of this fix.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::communication::channel::ble::clear_dial_exhaustion(&peer_device_id);

    let now = utc_now();

    let existing: Option<PairedDevice> = paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .optional()
        .map_err(|e| e.to_string())?;

    if let Some(_) = existing {
        diesel::update(paired_devices::table.find(&peer_device_id))
            .set((
                paired_devices::display_name.eq(&display_name),
                paired_devices::last_seen_at.eq(&now),
            ))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    } else {
        let input = CreatePairedDeviceInput {
            peer_device_id: peer_device_id.clone(),
            display_name: display_name.clone(),
            paired_at: now.clone(),
        };
        diesel::insert_into(paired_devices::table)
            .values(&input)
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    }

    // A pair's channels reflect how it was set up (ADR-0007). A person who
    // explicitly chose Bluetooth in the pairing dialog must not find Network
    // configured as well, with the app dialling the LAN over a channel they
    // never picked -- that would make the redesign's central promise false on
    // the very first screen.
    //
    // Outside the insert branch, on the same reasoning the Bluetooth block
    // below spells out: an existing row here means an asymmetric re-pair, the
    // other side having reset and paired again over the network while this
    // side kept its row. Configuring only on a fresh insert left that pair
    // with Network off or absent, so `check_channel_enabled` rejected the
    // auth while the dialog had just said pairing was complete. Completing a
    // pairing over a channel is a deliberate enough act to set that channel
    // up, which is exactly what the Bluetooth side already assumes.
    if !via_bluetooth {
        channels::configure(&mut *conn, &peer_device_id, ChannelKind::Network, true, None)?;
    }

    // ADR 0002 Phase 3: a Bluetooth address handed over as part of the
    // pairing handshake itself (either observed directly on a
    // Bluetooth-carried completion, or self-reported by the peer) is
    // recorded as diagnostics -- but only onto a channel that exists.
    //
    // Runs for both branches above, not just a fresh insert: this
    // function is only ever called as the final step of a real,
    // human-confirmed pairing completion, never from an unrelated
    // background path -- an *existing* row here means an asymmetric
    // re-pair (the other side reset and paired again while this side kept
    // its old row), and that fresh handshake's Bluetooth details are just
    // as real as a brand-new pair's. A completed BLE-first pairing is a
    // deliberate enough act to count as setting the channel up again, so
    // it overwrites a previous opt-out rather than being ignored by it --
    // otherwise the UI would report pairing complete while this side
    // permanently rejected every real session.
    let address = bluetooth_address.as_deref().and_then(normalize_bluetooth_address);
    if via_bluetooth {
        channels::configure(
            &mut *conn,
            &peer_device_id,
            ChannelKind::Bluetooth,
            true,
            address.as_deref(),
        )?;
    } else if let Some(address) = address.as_deref() {
        // An ordinary network pairing that happens to carry a self-reported
        // address is not evidence that Bluetooth works between these two.
        // Record where the peer says it can be found, and leave setting the
        // channel up to the person.
        channels::note_address(&mut *conn, &peer_device_id, ChannelKind::Bluetooth, address);
    }

    paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_save_paired_device(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
    display_name: String,
    bluetooth_address: Option<String>,
    via_bluetooth: bool,
) -> Result<PairedDevice, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_save_paired_device_impl(
        &mut conn,
        peer_device_id,
        display_name,
        bluetooth_address,
        via_bluetooth,
        state.db_path.clone(),
    )
}

/// The switch on a channel row: one function for both kinds, because
/// turning a channel off has to do the same four things either way, and
/// having had two of these is how the Network switch shipped greying the
/// row while traffic kept flowing.
///
/// Off means: the row is off, the primary is released, this device stops
/// dialling, and any session already open on the channel is closed. The
/// peer's inbound dial is refused separately, by the session gate
/// (`sync::session`), which is the other half of the same promise.
///
/// On means: the row is on and, for Bluetooth, the dial backoff is reset --
/// a person who just switched a channel on is asking for an attempt now,
/// not at the end of whatever window a previous failure opened.
///
/// Enabling never fails on a precondition. If the condition the channel
/// needs is absent -- no radio, peer away -- the channel stays on and the
/// row says so (ADR-0007).
pub fn device_connection_set_channel_enabled_impl(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: String,
    kind: ChannelKind,
    enabled: bool,
) -> Result<Vec<ChannelStatus>, String> {
    let paired: Option<PairedDevice> = paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .optional()
        .map_err(|e| e.to_string())?;
    if paired.is_none() {
        return Err("paired device not found".to_string());
    }

    if enabled && kind == ChannelKind::Bluetooth {
        // The one point in the app where requesting the runtime permission
        // triad is appropriate: an explicit switch flip, never startup and
        // never a background path (the dial loop, the peripheral acceptor).
        // See BluetoothPairing.requestPermissionsIfNeeded's doc comment.
        // Fire-and-forget: if the dialog is still unanswered, `hasPermissions`
        // below (correctly) fails closed and the same click can be retried.
        #[cfg(target_os = "android")]
        {
            crate::services::android_context::call_static_context_void(
                "com.fini.app.BluetoothPairing",
                "requestPermissionsIfNeeded",
            );
            if !crate::services::android_context::call_static_context_to_bool(
                "com.fini.app.BluetoothPairing",
                "hasPermissions",
            ) {
                return Err(
                    "Bluetooth permission required -- grant it in the dialog, then try again"
                        .to_string(),
                );
            }
        }
    }

    // `configure`, not `set_enabled`: flipping the switch on a channel that
    // has no row is how a channel gets set up in the first place, and the
    // page offers exactly that for a channel a pair has never used.
    // `set_enabled` on its own would update nothing and report success.
    if channels::find(&mut *conn, &peer_device_id, kind).is_some() {
        channels::set_enabled(&mut *conn, &peer_device_id, kind, enabled)?;
    } else {
        channels::configure(&mut *conn, &peer_device_id, kind, enabled, None)?;
    }

    if enabled {
        match kind {
            // Switching a channel on is one of the five moments work becomes
            // sendable (ADR-0007): the peer was filtered out of the dial loop
            // a moment ago and is eligible again. Without this the pair waits
            // for the hourly backstop to notice -- measured at 17s on
            // hardware for a reconnect the person just asked for and is
            // watching.
            ChannelKind::Network => {
                crate::services::communication::sync::commands::notify_sync_work_pending();
            }
            // Bluetooth's equivalent, plus a backoff reset: after a peer
            // exhausted its automatic retries, switching off and on again
            // used to return straight to `BluetoothDialExhausted` with
            // `spawn_dial_loop` still skipping it, ignoring the fresh
            // request entirely.
            ChannelKind::Bluetooth => {
                #[cfg(any(target_os = "linux", target_os = "android"))]
                crate::services::communication::channel::ble::retry_bluetooth_dial(
                    state,
                    &peer_device_id,
                );
                // The same wake Network does above, and on Android it is
                // what starts the foreground service: `start_sync_service_once`
                // only runs from inside a tick, so a first tick that happened
                // before Nearby Devices was granted left the service stopped.
                // Nothing else would start it -- the dial above supplies a
                // wake only if it reaches the peer, and "Turn on anyway"
                // exists precisely for when it cannot. The app would then be
                // frozen on backgrounding with Bluetooth sync switched on.
                crate::services::communication::sync::commands::notify_sync_work_pending();
            }
        }
    } else {
        // Closing the session is what makes the switch a switch. Without it
        // an already-connected session does not merely linger cosmetically:
        // it can still win primary-channel selection and have `push_to_peer`
        // resume real traffic over a channel the person just switched off.
        match kind {
            ChannelKind::Network => {
                state.close_session_on(&peer_device_id, ChannelKind::Network);
            }
            ChannelKind::Bluetooth => {
                state.close_session_on(&peer_device_id, ChannelKind::Bluetooth);
            }
        }
        // `channels::set_enabled` already released the primary in the DB;
        // this is the in-memory half. Without it a peer that happened to
        // have this channel primary keeps it primary until some unrelated
        // claim/release event triggers a recompute, rather than the instant
        // the switch takes effect.
        let pinned_to_bluetooth =
            channels::primary_kind(&mut *conn, &peer_device_id) == Some(ChannelKind::Bluetooth);
        let bluetooth_enabled =
            channels::is_enabled(&mut *conn, &peer_device_id, ChannelKind::Bluetooth);
        state.refresh_primary(&peer_device_id, pinned_to_bluetooth, bluetooth_enabled);
    }

    device_connection_channel_statuses_impl(conn, state, peer_device_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_set_channel_enabled(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
    kind: ChannelKind,
    enabled: bool,
) -> Result<Vec<ChannelStatus>, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_set_channel_enabled_impl(&mut conn, &state, peer_device_id, kind, enabled)
}

/// Forget a channel entirely: "unlink channel" on the Device page.
///
/// The pair keeps its trust, its mapped spaces and its other channel; this
/// one stops existing, and the page offers to set it up again from scratch.
/// Refused while the channel is on, so it is always a deliberate second act
/// rather than something one click can do to a working connection.
pub fn device_connection_unlink_channel_impl(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: String,
    kind: ChannelKind,
) -> Result<Vec<ChannelStatus>, String> {
    channels::unlink(&mut *conn, &peer_device_id, kind)?;
    device_connection_channel_statuses_impl(conn, state, peer_device_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_unlink_channel(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
    kind: ChannelKind,
) -> Result<Vec<ChannelStatus>, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_unlink_channel_impl(&mut conn, &state, peer_device_id, kind)
}

/// Choose which channel carries this pair's traffic: the star on a channel
/// row. `primary: None` clears the choice, falling back to the automatic
/// network-first rule.
///
/// The choice persists, so it governs future reconnects too, and the row
/// shows it whether or not that channel is connected right now (ADR-0007).
/// Selection is re-run immediately (`refresh_primary`) so the page reflects
/// it without waiting for a reconnect -- both channels stay connected
/// regardless of the choice, so there is nothing to switch or force-close,
/// only which already-connected one is primary.
///
/// Refused for a channel that is off: a channel that cannot connect cannot
/// carry the traffic, so honouring the choice would mean suppressing the
/// other one in favour of nothing.
pub fn device_connection_set_primary_channel_impl(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: String,
    primary: Option<ChannelKind>,
) -> Result<Vec<ChannelStatus>, String> {
    let paired: Option<PairedDevice> = paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .optional()
        .map_err(|e| e.to_string())?;
    if paired.is_none() {
        return Err("paired device not found".to_string());
    }
    if let Some(kind) = primary {
        if !channels::is_enabled(&mut *conn, &peer_device_id, kind) {
            return Err("Switch the channel on first".to_string());
        }
    }

    channels::set_primary(&mut *conn, &peer_device_id, primary)?;

    let pinned_to_bluetooth = primary == Some(ChannelKind::Bluetooth);
    let bluetooth_enabled =
        channels::is_enabled(&mut *conn, &peer_device_id, ChannelKind::Bluetooth);
    state.refresh_primary(&peer_device_id, pinned_to_bluetooth, bluetooth_enabled);

    device_connection_channel_statuses_impl(conn, state, peer_device_id)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_set_primary_channel(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
    primary: Option<ChannelKind>,
) -> Result<Vec<ChannelStatus>, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_set_primary_channel_impl(&mut conn, &state, peer_device_id, primary)
}

/// Whether this machine's Bluetooth radio can be used right now, asked
/// directly rather than inferred from the last background attempt.
///
/// The Device page calls this immediately after switching a Bluetooth
/// channel on, so a user whose own Bluetooth is off is told at the moment
/// they flip the switch instead of up to a tick later. `false` is not an
/// error and must not be shown as one: the channel stays on and starts by
/// itself once the radio comes back -- that is the whole point of the
/// "on, waiting" row state (`BluetoothStatusCode::AdapterOff`).
///
/// Deliberately not called on any polling path. It opens a real discovery
/// session; that is cheap once on a button press and wasteful every few
/// seconds, which is exactly why the passive signal exists alongside it.
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub async fn device_connection_probe_bluetooth_adapter() -> Result<bool, String> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        Ok(crate::services::communication::channel::ble::probe_adapter_available().await)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        // No BLE channel here at all, so there is no radio to be off.
        // The row reports `BluetoothNotSupported` on its own; answering
        // `false` would produce a toast blaming the user's hardware for a
        // platform decision.
        Ok(true)
    }
}

/// The "Find via Bluetooth" button on `DeviceView.vue` — Phase 1's discovery
/// mechanism (ADR 0002) for a peer that hasn't self-reported an address
/// (Android peers can't; see `local_bluetooth_address`'s doc comment) or
/// simply hasn't connected over network since this feature existed. Scans
/// for up to 60 seconds; `Ok(None)` means nothing matched in that window,
/// not an error -- the frontend shows "not found" rather than an error
/// state for that case. A genuine `Err` means Bluetooth itself couldn't be
/// used at all (no adapter, permission denied, etc).
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub async fn device_connection_find_bluetooth_address(
    state: State<'_, DeviceConnectionState>,
    peer_device_id: String,
) -> Result<Option<String>, String> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // Same click-triggered permission request as the "Enable Bluetooth"
        // toggle above -- this button is exactly the same class of genuine
        // user action, not a background/startup path. See
        // BluetoothPairing.requestPermissionsIfNeeded's doc comment.
        #[cfg(target_os = "android")]
        {
            crate::services::android_context::call_static_context_void(
                "com.fini.app.BluetoothPairing",
                "requestPermissionsIfNeeded",
            );
            if !crate::services::android_context::call_static_context_to_bool(
                "com.fini.app.BluetoothPairing",
                "hasPermissions",
            ) {
                return Err(
                    "Bluetooth permission required -- grant it in the dialog, then try again"
                        .to_string(),
                );
            }
        }

        let device_connection = state.inner().clone();
        let db_path = device_connection.db_path.clone();
        crate::services::communication::channel::ble::find_peer_address(
            device_connection,
            db_path,
            peer_device_id,
            Duration::from_secs(60),
        )
        .await
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = peer_device_id;
        Err("Bluetooth is not available on this platform".to_string())
    }
}

/// The pure in-memory half of `ChannelStatusInputs` -- everything
/// `has_session_on`/`primary_transport`/`channel_liveness_code` can
/// answer without a DB read or an OS-level check. Shared by
/// `device_connection_channel_statuses_impl` (which adds the DB-backed
/// preconditions on top) and `device_connection_channel_liveness_impl`
/// (which is *only* this -- the lightweight live-poll surface). Keeping
/// this in one place is what makes the two commands' Sim/Bluetooth kind
/// resolution impossible to drift apart.
struct ChannelLivenessSnapshot {
    network_connected: bool,
    network_primary: bool,
    network_code: Option<ChannelReason>,
    bluetooth_connected: bool,
    bluetooth_primary: bool,
    bluetooth_code: Option<ChannelReason>,
}

fn channel_liveness_snapshot(state: &DeviceConnectionState, peer_device_id: &str) -> ChannelLivenessSnapshot {
    // ADR-0003 revision: both channels can have a claimed session at
    // once now, so "the live channel" no longer exists as a single
    // value -- each row checks its own `has_session_on`.
    // One session slot per channel: the loopback radio reports Bluetooth like
    // any other way of connecting that channel, so there is no second
    // Bluetooth-ish kind to check for any more.
    let network_connected = state.has_session_on(peer_device_id, ChannelKind::Network);
    let bluetooth_connected = state.has_session_on(peer_device_id, ChannelKind::Bluetooth);

    let primary = state.primary_transport(peer_device_id);
    let network_primary = primary == Some(ChannelKind::Network);
    let bluetooth_primary = primary == Some(ChannelKind::Bluetooth);

    let network_code = network_connected
        .then(|| state.channel_liveness_code(peer_device_id, ChannelKind::Network))
        .flatten();
    let bluetooth_code = bluetooth_connected
        .then(|| state.channel_liveness_code(peer_device_id, ChannelKind::Bluetooth))
        .flatten();

    ChannelLivenessSnapshot {
        network_connected,
        network_primary,
        network_code,
        bluetooth_connected,
        bluetooth_primary,
        bluetooth_code,
    }
}

pub fn device_connection_channel_statuses_impl(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: String,
) -> Result<Vec<ChannelStatus>, String> {
    // Asked for its error, not its columns: a peer that is not paired has no
    // channels to report, and saying so beats returning two empty rows.
    paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first::<PairedDevice>(&mut *conn)
        .map_err(|e| e.to_string())?;
    // ADR-0006: no OS-bond lookup here any more. Dropping the bond check
    // also drops a `bluetoothctl` subprocess that used to run once per peer
    // on every status poll.
    let snapshot = channel_liveness_snapshot(state, &peer_device_id);
    let network = channels::find(&mut *conn, &peer_device_id, ChannelKind::Network);
    let bluetooth = channels::find(&mut *conn, &peer_device_id, ChannelKind::Bluetooth);

    let network_enabled = network.as_ref().is_some_and(|channel| channel.enabled);
    let bluetooth_enabled = bluetooth.as_ref().is_some_and(|channel| channel.enabled);

    // Each channel says why it cannot reach this peer. Nothing here knows
    // what a radio or a beacon is any more -- which is the point, because
    // the answers are about different machines and only the channel that
    // owns the mechanism can tell them apart honestly.
    let services = crate::services::communication::channel::service::services(state);
    let why_not = |kind: ChannelKind, enabled: bool| {
        services
            .iter()
            .find(|service| service.kind() == kind)
            .and_then(|service| service.why_not(&peer_device_id, enabled))
    };

    Ok(build_channel_statuses(ChannelStatusInputs {
        network_configured: network.is_some(),
        bluetooth_configured: bluetooth.is_some(),
        network_enabled,
        bluetooth_enabled,
        network_address: network.as_ref().and_then(|channel| channel.address.clone()),
        bluetooth_address: bluetooth.as_ref().and_then(|channel| channel.address.clone()),
        network_unconfigured_code: why_not(ChannelKind::Network, network_enabled),
        bluetooth_unconfigured_code: why_not(ChannelKind::Bluetooth, bluetooth_enabled),
        network_connected: snapshot.network_connected,
        bluetooth_connected: snapshot.bluetooth_connected,
        // The person's stored choice, not `snapshot`'s live primary: the
        // star is a setting (ADR-0007), and reading it off the live value
        // would make it move on its own whenever a link dropped.
        network_primary: network.as_ref().is_some_and(|channel| channel.is_primary),
        bluetooth_primary: bluetooth.as_ref().is_some_and(|channel| channel.is_primary),
        network_code: snapshot.network_code,
        bluetooth_code: snapshot.bluetooth_code,
    }))
}

/// The Device page's "click the Bluetooth row to try again" affordance
/// (see `BluetoothStatusCode::DialExhausted`'s doc comment): a
/// no-op everywhere the dial loop wasn't exhausted, so the frontend doesn't
/// need to guard the call. Takes only `state`, not `db` -- `ble::
/// retry_bluetooth_dial` looks up the peer's dial address itself, off its
/// own DB connection, on the task it spawns; see its own doc comment for why
/// this command doesn't (and, per a P2 review finding on an earlier revision
/// of this command, must not) touch the shared `AppDbConnection` at all.
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_retry_bluetooth_dial(state: State<DeviceConnectionState>, peer_device_id: String) -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        crate::services::communication::channel::ble::retry_bluetooth_dial(&state, &peer_device_id);
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (state, peer_device_id);
    }
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_channel_statuses(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
) -> Result<Vec<ChannelStatus>, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_channel_statuses_impl(&mut conn, &state, peer_device_id)
}

/// Lightweight sibling of `device_connection_channel_statuses`: no DB
/// read, no OS-level bond check, just `has_session_on`/`primary_transport`/
/// `channel_liveness_code` for each row. Meant to be polled far more
/// often than the heavy version -- see `ChannelLiveness`'s own doc
/// comment for the P1 review finding this exists to fix (the live poll
/// previously only refreshed `primary`, leaving green/amber frozen).
pub fn device_connection_channel_liveness_impl(
    state: &DeviceConnectionState,
    peer_device_id: String,
) -> Vec<ChannelLiveness> {
    let snapshot = channel_liveness_snapshot(state, &peer_device_id);
    vec![
        ChannelLiveness {
            kind: ChannelKind::Network,
            connected: snapshot.network_connected,
            reason: snapshot.network_code.map(|r| r.code().to_string()),
            dial_exhausted: false,
        },
        ChannelLiveness {
            kind: ChannelKind::Bluetooth,
            connected: snapshot.bluetooth_connected,
            reason: snapshot.bluetooth_code.map(|r| r.code().to_string()),
            dial_exhausted: crate::services::communication::channel::service::service_for(
                state,
                ChannelKind::Bluetooth,
            )
            .dial_exhausted(&peer_device_id),
        },
    ]
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_channel_liveness(
    state: State<DeviceConnectionState>,
    peer_device_id: String,
) -> Vec<ChannelLiveness> {
    device_connection_channel_liveness_impl(&state, peer_device_id)
}

/// Every paired peer eligible for a Bluetooth dial attempt right now: the
/// ones whose Bluetooth channel is set up and on. Used by
/// `channel::ble::spawn_dial_loop` — unlike `tcp_ws`/`sim` there is no
/// presence worker or static port list to draw candidates from.
///
/// ADR-0006: this used to also require a stored address and a live OS bond,
/// and to return the address to dial. It returns peer ids alone now, because
/// there is no address to dial *to* — the dialer finds the peer by scanning
/// for Fini's service UUID and proves who answered with the `Auth` frame.

pub fn bluetooth_dial_candidates(conn: &mut SqliteConnection) -> Vec<String> {
    channels::peers_with_channel_enabled(conn, ChannelKind::Bluetooth)
}

/// Records the link-layer address a peer was actually reached at, purely so
/// hardware logs and the Device view can show it. ADR-0006: nothing dials
/// this value any more, and nothing gates on it — a peer that advertises
/// under a rotating address (every modern Android) will simply rewrite it
/// each time, which is expected rather than a problem to solve.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn note_observed_bluetooth_address(conn: &mut SqliteConnection, peer_id: &str, address: &str) {
    channels::note_address(conn, peer_id, ChannelKind::Bluetooth, address);
}

/// The channel this pair's traffic was pinned to, or `None` for automatic
/// (network-first) selection. Both channels dial and connect regardless of
/// it (ADR-0003 revision) -- it decides only which already-connected one
/// counts as primary. A missing/unpaired row reads as no choice, matching
/// every other `unwrap_or_default`-style read in this module.
pub fn peer_primary_channel(conn: &mut SqliteConnection, peer_id: &str) -> Option<ChannelKind> {
    channels::primary_kind(conn, peer_id)
}

/// The Bluetooth switch for this pair. Used by
/// `DeviceConnectionState::bluetooth_primary_eligibility` to exclude a
/// switched-off pair's Bluetooth session from primary-channel candidacy
/// (see `recompute_primary_locked`'s own doc comment for the P1 review
/// finding this closes). A pair with no Bluetooth channel configured reads
/// as off, failing closed the same direction the session gate does.
pub fn peer_bluetooth_enabled(conn: &mut SqliteConnection, peer_id: &str) -> bool {
    channels::is_enabled(conn, peer_id, ChannelKind::Bluetooth)
}

pub fn device_connection_session_channel_impl(
    state: &DeviceConnectionState,
    peer_device_id: String,
) -> Option<ChannelKind> {
    state.primary_transport(&peer_device_id)
}

/// Which channel (if any) is currently primary for a peer. Debug/test
/// surface proving per-channel claiming end-to-end through the real app
/// binary — see `specs/e2e/actors/tests/peer-sync-over-sim.spec.ts`.
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_session_channel(
    state: State<DeviceConnectionState>,
    peer_device_id: String,
) -> Option<ChannelKind> {
    device_connection_session_channel_impl(&state, peer_device_id)
}

pub fn device_connection_unpair_impl(
    conn: &mut SqliteConnection,
    peer_device_id: String,
) -> Result<(), String> {
    diesel::delete(paired_devices::table.find(&peer_device_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    // See `device_connection_save_paired_device_impl`'s matching comment --
    // this is the unpair-side half of the same P2 review finding.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::communication::channel::ble::clear_dial_exhaustion(&peer_device_id);
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_unpair(
    db: State<AppDbConnection>,
    peer_device_id: String,
) -> Result<(), String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_unpair_impl(&mut conn, peer_device_id)
}

pub fn device_connection_update_last_seen_impl(
    conn: &mut SqliteConnection,
    peer_device_id: String,
    last_seen_at: String,
) -> Result<(), String> {
    diesel::update(paired_devices::table.find(&peer_device_id))
        .set(paired_devices::last_seen_at.eq(&last_seen_at))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_update_last_seen(
    db: State<AppDbConnection>,
    peer_device_id: String,
    last_seen_at: String,
) -> Result<(), String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_update_last_seen_impl(&mut conn, peer_device_id, last_seen_at)
}

/// `FINI_BLUETOOTH_PAIRED_ADDRESSES` is process-global; tests that set and
/// clear it must not interleave with each other under the default
/// parallel test runner. `pub(crate)` (not nested inside `mod tests`
/// below) and shared with `channel::tests`, which independently sets/
/// clears the same env var in its own tests: two separate locks for one
/// shared mutable global meant they could still race with *each other*
/// across files, observed as intermittent failures once enough tests in
/// both modules touched it.
#[cfg(test)]
pub(crate) static BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db;

    use super::BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK as ENV_LOCK;

    /// Regression test: `run_command_with_timeout` must actually terminate
    /// a command that outlives its deadline, not just stop waiting on it --
    /// `sleep 30` run with a 200ms timeout proves this by returning `None`
    /// almost immediately rather than only after the full 30 seconds.
    #[tokio::test]
    async fn run_command_with_timeout_returns_promptly_on_a_hung_command() {
        let mut command = tokio::process::Command::new("sleep");
        command.arg("30");

        let started = std::time::Instant::now();
        let result = run_command_with_timeout(command, Duration::from_millis(200)).await;
        let elapsed = started.elapsed();

        assert!(result.is_none());
        assert!(
            elapsed < Duration::from_secs(5),
            "must return promptly once the timeout elapses, took {elapsed:?}"
        );
    }

    fn test_conn() -> (SqliteConnection, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);
        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-a"),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
            ))
            .execute(&mut conn)
            .expect("insert paired device");
        (conn, db_path)
    }

    /// A `DeviceConnectionState` for the switch/primary commands, which act
    /// on live sessions as well as rows. No sessions are ever claimed here,
    /// so `close_session_on`/`refresh_primary` are no-ops and what the tests
    /// observe is purely what was written.
    fn test_state() -> (SqliteConnection, DeviceConnectionState) {
        let (conn, db_path) = test_conn();
        let data_dir = db_path.with_extension("data");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        // Keeps these hermetic: no real mDNS daemon.
        std::env::set_var("FINI_MDNS_DISABLED", "1");
        let state = DeviceConnectionState::from_db_path(&data_dir, db_path);
        (conn, state)
    }

    fn bluetooth_row(conn: &mut SqliteConnection) -> Option<crate::models::Channel> {
        channels::find(conn, "peer-a", ChannelKind::Bluetooth)
    }

    /// A new pair's channels reflect how it was set up (ADR-0007). Pairing
    /// over Bluetooth configures Bluetooth and nothing else -- a person who
    /// chose Bluetooth in the pairing dialog must not find the app dialling
    /// the LAN over a channel they never picked.
    #[test]
    fn save_paired_device_sets_up_only_the_channel_the_pairing_arrived_over() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        device_connection_save_paired_device_impl(
            &mut conn,
            "peer-new".to_string(),
            "Peer New".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
            true, // via_bluetooth
            db_path.clone(),
        )
        .expect("save paired device");

        let bluetooth = channels::find(&mut conn, "peer-new", ChannelKind::Bluetooth)
            .expect("the Bluetooth channel the pairing arrived over");
        assert!(bluetooth.enabled);
        assert_eq!(
            bluetooth.address.as_deref(),
            Some("AA:BB:CC:DD:EE:FF"),
            "the address the handshake carried is recorded, as diagnostics"
        );
        assert!(
            channels::find(&mut conn, "peer-new", ChannelKind::Network).is_none(),
            "Network must not be configured by a Bluetooth pairing"
        );

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// The asymmetric re-pair: this side kept its row while the other side
    /// reset and paired again over the network. Configuring Network only on a
    /// fresh insert left the channel off or absent here, so the dialog said
    /// pairing was complete and `check_channel_enabled` then rejected every
    /// auth -- a pair that looks set up and cannot talk.
    ///
    /// The Bluetooth side has always treated a completed handshake as
    /// deliberate enough to set its channel up again; this asserts Network
    /// does too.
    #[test]
    fn re_pairing_over_the_network_sets_network_up_again_on_an_existing_pair() {
        let _guard = ENV_LOCK.lock().unwrap();

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        device_connection_save_paired_device_impl(
            &mut conn,
            "peer-old".to_string(),
            "Peer Old".to_string(),
            None,
            false, // via_bluetooth
            db_path.clone(),
        )
        .expect("first pairing");

        // The person switched Network off at some point after pairing.
        channels::set_enabled(&mut conn, "peer-old", ChannelKind::Network, false)
            .expect("switch Network off");
        assert!(!channels::is_enabled(&mut conn, "peer-old", ChannelKind::Network));

        // The other side reset and paired again, over the network.
        device_connection_save_paired_device_impl(
            &mut conn,
            "peer-old".to_string(),
            "Peer Old".to_string(),
            None,
            false, // via_bluetooth
            db_path.clone(),
        )
        .expect("re-pairing");

        assert!(
            channels::is_enabled(&mut conn, "peer-old", ChannelKind::Network),
            "a completed Network pairing must leave the Network channel usable"
        );
    }

    /// The counterpart: an ordinary network pairing configures Network, and
    /// a self-reported Bluetooth address alongside it is not evidence that
    /// Bluetooth works between these two devices. It is dropped rather than
    /// stored, because storing it would mean a Bluetooth row -- which is the
    /// same thing as offering a channel the person never asked for.
    #[test]
    fn save_paired_device_over_the_network_does_not_set_bluetooth_up() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        device_connection_save_paired_device_impl(
            &mut conn,
            "peer-network-paired".to_string(),
            "Peer Network Paired".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
            false, // via_bluetooth
            db_path.clone(),
        )
        .expect("save paired device");

        let network = channels::find(&mut conn, "peer-network-paired", ChannelKind::Network)
            .expect("the Network channel the pairing arrived over");
        assert!(network.enabled);
        assert!(
            channels::find(&mut conn, "peer-network-paired", ChannelKind::Bluetooth).is_none(),
            "a self-reported address must not set a second channel up"
        );

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// An asymmetric re-pair -- the other side reset and paired again while
    /// this side kept its row -- can land on a channel this device had
    /// switched off. Completing a whole BLE-first pairing is a deliberate
    /// enough act to set it back up; without this the UI would report
    /// pairing complete while this side permanently rejected every real
    /// session.
    #[test]
    fn a_fresh_bluetooth_pairing_switches_a_previously_switched_off_channel_back_on() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let (mut conn, db_path) = test_conn();
        channels::configure(&mut conn, "peer-a", ChannelKind::Bluetooth, false, None)
            .expect("seed a channel that was set up and switched off");

        device_connection_save_paired_device_impl(
            &mut conn,
            "peer-a".to_string(),
            "Peer A".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
            true, // via_bluetooth
            db_path,
        )
        .expect("save paired device");

        let row = bluetooth_row(&mut conn).expect("the Bluetooth row");
        assert!(
            row.enabled,
            "a fresh Bluetooth pairing must not be silently ignored by an earlier switch-off"
        );
        assert_eq!(row.address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// A self-report records where the peer says it can be found, and never
    /// moves the switch in either direction.
    ///
    /// The OS bond used to decide this, which made a background message able
    /// to turn a channel on or off. Off was the damaging direction: during
    /// hardware verification the desktop reported its address, the phone
    /// found no bond, and switched off the very channel that was about to
    /// connect. What reached the other side was `auth rejected: bluetooth
    /// disabled for this pair`, with nothing anywhere naming the cause.
    #[test]
    fn a_self_report_records_the_address_and_never_moves_the_switch() {
        let _guard = ENV_LOCK.lock().unwrap();
        // A *confirmed* not-paired result, not merely an absent one:
        // `remove_var` alone falls through to the real `bluetoothctl`, which
        // is not deterministic here.
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

        let (mut conn, _db_path) = test_conn();
        channels::configure(
            &mut conn,
            "peer-a",
            ChannelKind::Bluetooth,
            true,
            Some("AA:BB:CC:DD:EE:FF"),
        )
        .expect("seed a channel that is on");

        let switched_on_by_this_call =
            persist_bluetooth_address_and_maybe_enable(&mut conn, "peer-a", "11:22:33:44:55:66")
                .expect("persist bluetooth address");
        assert!(
            !switched_on_by_this_call,
            "a self-report never switches a channel on by itself"
        );

        let row = bluetooth_row(&mut conn).expect("the Bluetooth row");
        assert_eq!(row.address.as_deref(), Some("11:22:33:44:55:66"));
        assert!(
            row.enabled,
            "an unbonded self-report must not switch off a channel the person switched on"
        );

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// The other half: a channel that was set up and switched off stays
    /// exactly as the person left it, address included. This is the job
    /// `bluetooth_disabled_by_user` used to hold a whole column for -- a row
    /// that exists and is off says the same thing, and cannot fall out of
    /// step with the switch beside it.
    #[test]
    fn a_self_report_leaves_a_switched_off_channel_alone() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let (mut conn, _db_path) = test_conn();
        channels::configure(&mut conn, "peer-a", ChannelKind::Bluetooth, false, None)
            .expect("seed a channel that was switched off");

        persist_bluetooth_address_and_maybe_enable(&mut conn, "peer-a", "AA:BB:CC:DD:EE:FF")
            .expect("persist bluetooth address");

        let row = bluetooth_row(&mut conn).expect("the Bluetooth row");
        assert!(!row.enabled, "an explicit switch-off must survive a self-report");
        assert_eq!(
            row.address, None,
            "and the self-report must not quietly repopulate what it left"
        );

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// ADR-0006: switching Bluetooth on demands neither a stored address nor
    /// a live OS bond. Both used to be required, and since a peer is only
    /// ever dialled over a channel that is on, requiring a bond here made
    /// the bondless dial path unreachable in practice.
    #[tokio::test(flavor = "multi_thread")]
    async fn switching_a_channel_on_needs_neither_an_address_nor_a_bond() {
        let _guard = ENV_LOCK.lock().unwrap();
        // An allow-list matching nothing: no address here is bonded.
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

        let (mut conn, state) = test_state();
        let statuses = device_connection_set_channel_enabled_impl(
            &mut conn,
            &state,
            "peer-a".to_string(),
            ChannelKind::Bluetooth,
            true,
        )
        .expect("switching on without any address must succeed");

        let row = statuses
            .iter()
            .find(|status| status.kind == ChannelKind::Bluetooth)
            .expect("a Bluetooth row");
        assert!(row.configured, "the switch is what sets a channel up");
        assert!(row.enabled);

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// Switching a channel off is not forgetting it (ADR-0007): what it
    /// learned is kept, so switching it back on does not start from nothing.
    /// Forgetting is a separate act -- unlink.
    #[tokio::test(flavor = "multi_thread")]
    async fn switching_a_channel_off_keeps_what_it_learned() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let (mut conn, state) = test_state();
        channels::configure(
            &mut conn,
            "peer-a",
            ChannelKind::Bluetooth,
            true,
            Some("AA:BB:CC:DD:EE:FF"),
        )
        .expect("set the channel up with an address");

        device_connection_set_channel_enabled_impl(
            &mut conn,
            &state,
            "peer-a".to_string(),
            ChannelKind::Bluetooth,
            false,
        )
        .expect("switch bluetooth off");

        let row = bluetooth_row(&mut conn).expect("the row must survive being switched off");
        assert!(!row.enabled);
        assert_eq!(
            row.address.as_deref(),
            Some("AA:BB:CC:DD:EE:FF"),
            "switching off is not forgetting -- turning it back on must not start from nothing"
        );

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// Regression test for a P1 review finding on ADR-0003: a primary choice
    /// must not survive that channel being switched off -- the other one
    /// would stand down for a choice that can no longer be honoured,
    /// stranding the pair on no channel at all.
    #[tokio::test(flavor = "multi_thread")]
    async fn switching_a_channel_off_releases_its_primary() {
        let (mut conn, state) = test_state();
        channels::configure(&mut conn, "peer-a", ChannelKind::Bluetooth, true, None)
            .expect("set bluetooth up");
        channels::set_primary(&mut conn, "peer-a", Some(ChannelKind::Bluetooth))
            .expect("choose it as primary");

        device_connection_set_channel_enabled_impl(
            &mut conn,
            &state,
            "peer-a".to_string(),
            ChannelKind::Bluetooth,
            false,
        )
        .expect("switch bluetooth off");

        assert_eq!(
            channels::primary_kind(&mut conn, "peer-a"),
            None,
            "a primary choice must not survive its own channel being switched off"
        );
    }

    /// Sibling of the test above: switching one channel off must leave a
    /// primary choice made on the *other* one alone. Only the choice that
    /// can no longer be honoured is a stranding hazard.
    #[tokio::test(flavor = "multi_thread")]
    async fn switching_a_channel_off_leaves_the_other_ones_primary_alone() {
        let (mut conn, state) = test_state();
        channels::configure(&mut conn, "peer-a", ChannelKind::Network, true, None)
            .expect("set network up");
        channels::configure(&mut conn, "peer-a", ChannelKind::Bluetooth, true, None)
            .expect("set bluetooth up");
        channels::set_primary(&mut conn, "peer-a", Some(ChannelKind::Network))
            .expect("choose network as primary");

        device_connection_set_channel_enabled_impl(
            &mut conn,
            &state,
            "peer-a".to_string(),
            ChannelKind::Bluetooth,
            false,
        )
        .expect("switch bluetooth off");

        assert_eq!(
            channels::primary_kind(&mut conn, "peer-a"),
            Some(ChannelKind::Network)
        );
    }

    /// Unlink is the deliberate second act: a channel that is still on
    /// cannot be forgotten by one click, and the refusal says what to do
    /// instead.
    #[tokio::test(flavor = "multi_thread")]
    async fn unlink_refuses_while_the_channel_is_still_on() {
        let (mut conn, state) = test_state();
        channels::configure(&mut conn, "peer-a", ChannelKind::Bluetooth, true, None)
            .expect("set bluetooth up");

        let err = device_connection_unlink_channel_impl(
            &mut conn,
            &state,
            "peer-a".to_string(),
            ChannelKind::Bluetooth,
        )
        .expect_err("must refuse to unlink a channel that is on");
        assert!(err.contains("Turn the channel off first"), "got: {err}");
        assert!(bluetooth_row(&mut conn).is_some());
    }

    /// And once it is off, unlinking forgets it entirely -- the page goes
    /// back to offering to set it up.
    #[tokio::test(flavor = "multi_thread")]
    async fn unlink_forgets_a_switched_off_channel() {
        let (mut conn, state) = test_state();
        channels::configure(
            &mut conn,
            "peer-a",
            ChannelKind::Bluetooth,
            false,
            Some("AA:BB:CC:DD:EE:FF"),
        )
        .expect("set bluetooth up, switched off");

        let statuses = device_connection_unlink_channel_impl(
            &mut conn,
            &state,
            "peer-a".to_string(),
            ChannelKind::Bluetooth,
        )
        .expect("unlink a switched-off channel");

        assert!(bluetooth_row(&mut conn).is_none());
        let row = statuses
            .iter()
            .find(|status| status.kind == ChannelKind::Bluetooth)
            .expect("a Bluetooth row is still reported, as an offer to set it up");
        assert!(!row.configured);
        assert!(!row.enabled);
    }

    #[test]
    fn unpair_removes_the_paired_device_row_and_its_channels() {
        let (mut conn, _db_path) = test_conn();
        channels::configure(&mut conn, "peer-a", ChannelKind::Bluetooth, true, None)
            .expect("set bluetooth up");

        device_connection_unpair_impl(&mut conn, "peer-a".to_string()).expect("unpair");

        let remaining = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first::<PairedDevice>(&mut conn)
            .optional()
            .expect("query after unpair");
        assert!(remaining.is_none(), "unpair must remove the row");
        assert!(
            bluetooth_row(&mut conn).is_none(),
            "and its channels must go with it -- ON DELETE CASCADE, not a second delete to forget"
        );
    }

    /// The "unpair, then pair again" lifecycle a person takes when resetting
    /// a stuck pair. Unpair deletes the row outright (unlike switching a
    /// channel off, which keeps it), so the re-pair always goes through
    /// `save_paired_device`'s *insert* branch. Proves the full cycle rather
    /// than each half in isolation: nothing leaks from the deleted row, and
    /// the fresh pairing sets its channel up normally.
    #[test]
    fn unpair_then_re_pair_via_bluetooth_starts_with_clean_state() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let (mut conn, _db_path) = test_conn();
        channels::configure(
            &mut conn,
            "peer-a",
            ChannelKind::Bluetooth,
            true,
            Some("AA:BB:CC:DD:EE:FF"),
        )
        .expect("set bluetooth up on the original pairing");

        device_connection_unpair_impl(&mut conn, "peer-a".to_string()).expect("unpair");

        // A real re-pair hands over whatever address the fresh handshake
        // observed -- plausibly a different one (the peer may have paired
        // again from a different adapter or OS install).
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "11:22:33:44:55:66");
        device_connection_save_paired_device_impl(
            &mut conn,
            "peer-a".to_string(),
            "Peer A".to_string(),
            Some("11:22:33:44:55:66".to_string()),
            true, // via_bluetooth
            std::path::PathBuf::from("/nonexistent"),
        )
        .expect("re-pair after unpair");

        let row = bluetooth_row(&mut conn).expect("the re-paired Bluetooth row");
        assert!(row.enabled, "a fresh re-pair sets its channel up normally");
        assert_eq!(row.address.as_deref(), Some("11:22:33:44:55:66"));

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }
}
