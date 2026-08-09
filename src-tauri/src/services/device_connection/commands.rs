use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use futures_util::SinkExt;
use std::net::IpAddr;
use std::time::Duration;
#[cfg(any(feature = "ui-plane", test))]
use tauri::State;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::{DISCOVERY_PROTOCOL, DISCOVERY_TTL_SECS, PAIR_REQUEST_TTL_SECS};
use crate::models::{CreatePairedDeviceInput, PairedDevice};
use crate::schema::paired_devices;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
use crate::services::device_connection::runtime::{
    generate_passcode, prune_expired_incoming_requests, utc_now,
};
use crate::services::device_connection::types::{
    DeviceBluetoothTransportInput, DeviceConnectionDebugStatus, DeviceIdentity,
    DevicePairRequestAckInput, DevicePairRequestBluetoothInput, DevicePairRequestInput,
    DiscoveredDevice, IncomingPairRequest, IncomingSpaceMappingUpdate, PairAcceptPayload,
    PairCodeUpdate, PairCompletePayload, PairCompletionUpdate, PairRequestPayload,
};
use crate::services::device_connection::DeviceConnectionState;
use crate::services::device_connection::{build_transport_statuses, TransportStatus};
use crate::services::space_sync::types::PeerFrame;
use crate::services::transport::TransportKind;

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

pub(crate) fn bluetooth_address_is_os_paired(address: &str) -> bool {
    if let Ok(allowed) = std::env::var("FINI_BLUETOOTH_PAIRED_ADDRESSES") {
        return allowed
            .split(',')
            .filter_map(normalize_bluetooth_address)
            .any(|item| item == address);
    }

    #[cfg(target_os = "linux")]
    {
        // Inside the Flatpak sandbox, `bluetoothctl` and the system D-Bus
        // it needs aren't reachable directly (the GNOME runtime doesn't
        // bundle the binary, and its own D-Bus proxy is per-app/session,
        // not the host's `bluetoothd`). Route through `flatpak-spawn
        // --host` instead, the same pattern already used elsewhere in this
        // codebase (see `lib.rs`'s `FLATPAK_ID` check) — it runs the
        // command on the host, where the real `bluetoothctl` and its
        // system-bus connection exist, using the `--talk-name=org.freedesktop.Flatpak`
        // permission the manifest already grants.
        let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
            let mut command = std::process::Command::new("flatpak-spawn");
            command.arg("--host").arg("bluetoothctl");
            command
        } else {
            std::process::Command::new("bluetoothctl")
        };
        return command
            .arg("info")
            .arg(address)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|stdout| stdout.lines().any(|line| line.trim() == "Paired: yes"))
            .unwrap_or(false);
    }

    #[cfg(target_os = "android")]
    {
        // `BluetoothPairing.isBonded` (com.fini.app, not ble-gatt's
        // dev.blegatt bridge — OS bond status is a Fini pairing
        // precondition, not part of the reusable GATT transport surface,
        // mirroring the Linux branch above shelling out to `bluetoothctl`
        // directly rather than through `ble_gatt::backend::linux`) queries
        // `BluetoothAdapter.getBondedDevices()` via JNI.
        return crate::services::android_context::call_static_context_string_to_bool(
            "com.fini.app.BluetoothPairing",
            "isBonded",
            address,
        );
    }

    #[allow(unreachable_code)]
    false
}

/// This device's own real Bluetooth adapter address, when the platform can
/// read it at all. `None` on Android: `BluetoothAdapter.getAddress()` has
/// returned a dummy value since Android 6.0 for every normal app (a
/// permanent platform privacy protection, not a bug to work around) — so
/// Android has nothing real to self-report over `PeerFrame::BluetoothAddressUpdate`
/// and instead is discovered by a peer's BLE scan (`transport::ble`'s
/// discovery path). Linux has no such restriction.
pub(crate) fn local_bluetooth_address() -> Option<String> {
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
            let mut command = std::process::Command::new("flatpak-spawn");
            command.arg("--host").arg("bluetoothctl");
            command
        } else {
            std::process::Command::new("bluetoothctl")
        };
        return command
            .arg("show")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|stdout| {
                stdout.lines().find_map(|line| {
                    let rest = line.trim().strip_prefix("Controller ")?;
                    normalize_bluetooth_address(rest.split_whitespace().next()?)
                })
            });
    }

    #[allow(unreachable_code)]
    None
}

/// Stores `address` as `peer_id`'s Bluetooth address, and additionally
/// enables Bluetooth for the pair if -- and only if -- `address` is
/// currently OS-bonded *on this machine*. Returns whether it was enabled.
///
/// Shared by both Phase 1 mechanisms of ADR 0002: `session::run_session`'s
/// inbound `BluetoothAddressUpdate` handler (self-report) and
/// `transport::ble`'s scan-and-auth discovery. Both already have a form of
/// remote confirmation before calling this (an authenticated `PeerFrame`
/// channel, or a live `AuthOk` from the discovered address) -- what neither
/// proves on its own is that *this* device has completed OS-level bonding
/// with that address. `check_bluetooth_bond` (space_sync::session) already
/// enforces that only the *accepting* side of a Bluetooth session, so a
/// remote AuthOk during discovery only proves bonding on the *other*
/// side. Requiring it here too, symmetrically, before auto-enabling keeps
/// this consistent with `device_connection_set_bluetooth_transport_impl`'s
/// own manual-enable precondition -- never silently enable a pair this
/// machine can't actually use yet.
pub(crate) fn persist_bluetooth_address_and_maybe_enable(
    conn: &mut SqliteConnection, peer_id: &str, address: &str,
) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let is_os_paired = bluetooth_address_is_os_paired(address);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let is_os_paired = false;

    if is_os_paired {
        let _ = diesel::update(paired_devices::table.find(peer_id))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some(address)),
                paired_devices::bluetooth_last_verified_at
                    .eq(Some(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string())),
            ))
            .execute(conn);
    } else {
        let _ = diesel::update(paired_devices::table.find(peer_id))
            .set(paired_devices::bluetooth_address.eq(Some(address)))
            .execute(conn);
    }
    is_os_paired
}

/// One-shot pre-auth pairing sender (`PairRequest`/`PairAccept`/`PairComplete`).
/// Independent of `transport::tcp_ws::TcpWsLink` (connect, send one frame,
/// close — no need for a full `Link`), but MUST encode via the same
/// `transport::codec::encode_frame` (envelope-wrapped) and the same `Message::Text`
/// framing `TcpWsLink` reads, or `run_peer_gate` silently fails to parse the
/// first frame.
fn send_pair_ws(addr: IpAddr, port: u16, msg: PeerFrame) -> Result<(), String> {
    tauri::async_runtime::block_on(async move {
        let url = ws_url(addr, port);
        let (mut ws, _) = connect_async(&url)
            .await
            .map_err(|err| format!("connect pair websocket {url} failed: {err}"))?;
        let bytes = crate::services::transport::codec::encode_frame(&msg)
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
/// dance needed here: `transport::send_frame` already handles encoding for
/// any `Link`, unlike the WebSocket path, which has to hand-roll a
/// `Message::Text` frame around the same codec.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn send_pair_ble(address: &str, msg: PeerFrame) -> Result<(), String> {
    tauri::async_runtime::block_on(async move {
        let mut link = crate::services::transport::ble::dial(address).await?;
        crate::services::transport::send_frame(link.as_mut(), &msg).await
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
    // One toggle, both transports (ADR 0002 Phase 3): entering add-mode
    // makes this device discoverable over Bluetooth too, not just the
    // existing mDNS beacon.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::transport::ble::set_add_mode(true);
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
    crate::services::transport::ble::set_add_mode(false);
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
/// which transport carried it, so nothing downstream of `send_pair_ble`
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
/// `transport: Bluetooth`, for the unified candidate list.
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
        // Same reasoning as `device_connection_find_bluetooth_address_impl`'s
        // permission block above: opening Add Device mode is a genuine user
        // action, not a background/startup path, so this is an appropriate
        // point to prompt. Without this, a first-ever Add Device scan on
        // Android 12+ silently returns nothing and the frontend's scan loop
        // gives up for the rest of the session -- see
        // `BluetoothPairing.requestPermissionsIfNeeded`'s doc comment.
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

        let my_device_id = state.identity.device_id.clone();
        let candidates = crate::services::transport::ble::scan_add_mode_candidates(
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
                transport: crate::services::device_connection::transport::TransportKind::Bluetooth,
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
        // Best-effort: shared regardless of which transport carries this
        // frame, so a network-carried completion can still hand the
        // requester a Bluetooth address to store (ADR 0002 Phase 3).
        bluetooth_address: local_bluetooth_address(),
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
            transport: Default::default(),
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
            transport: Default::default(),
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
) -> Result<PairedDevice, String> {
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

        // ADR 0002 Phase 3: a Bluetooth address handed over as part of the
        // pairing handshake itself (either observed directly on a
        // Bluetooth-carried completion, or self-reported by the peer) is
        // trusted immediately -- unlike the Device settings toggle
        // (`device_connection_set_bluetooth_transport_impl`), this isn't a
        // background path and a real pairing handshake with human code
        // confirmation just proved the peers are who they claim to be, so
        // it doesn't need the OS-bond/permission gate that toggle enforces.
        if let Some(address) = bluetooth_address.as_deref().and_then(normalize_bluetooth_address) {
            diesel::update(paired_devices::table.find(&peer_device_id))
                .set((
                    paired_devices::bluetooth_enabled.eq(true),
                    paired_devices::bluetooth_address.eq(Some(address)),
                    paired_devices::bluetooth_last_verified_at.eq(Some(now.clone())),
                ))
                .execute(&mut *conn)
                .map_err(|e| e.to_string())?;
        }
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
    peer_device_id: String,
    display_name: String,
    bluetooth_address: Option<String>,
) -> Result<PairedDevice, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_save_paired_device_impl(&mut conn, peer_device_id, display_name, bluetooth_address)
}

pub fn device_connection_set_bluetooth_transport_impl(
    conn: &mut SqliteConnection,
    input: DeviceBluetoothTransportInput,
) -> Result<PairedDevice, String> {
    let existing: Option<PairedDevice> = paired_devices::table
        .find(&input.peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.is_none() {
        return Err("paired device not found".to_string());
    }

    let normalized_address = input
        .bluetooth_address
        .as_deref()
        .and_then(normalize_bluetooth_address);

    if input.enabled {
        let Some(address) = normalized_address else {
            return Err("bluetooth address is required to enable Bluetooth transport".to_string());
        };

        // This command only runs from the user explicitly flipping the
        // Bluetooth toggle in Device settings -- the one point in the app
        // where requesting the runtime permission triad is appropriate. Not
        // requested at startup or from any background path (the dial loop,
        // the peripheral acceptor): see BluetoothPairing.requestPermissionsIfNeeded's
        // doc comment. Fire-and-forget: if the user hasn't responded to the
        // dialog yet, `isBonded`/`hasPermissions` below still (correctly)
        // fail closed, and this same toggle click can just be retried once
        // they grant it.
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

        if !bluetooth_address_is_os_paired(&address) {
            return Err(
                "OS Bluetooth pairing is required before enabling Bluetooth transport".to_string(),
            );
        }

        diesel::update(paired_devices::table.find(&input.peer_device_id))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some(address)),
                paired_devices::bluetooth_last_verified_at.eq(Some(utc_now())),
            ))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    } else {
        diesel::update(paired_devices::table.find(&input.peer_device_id))
            .set((
                paired_devices::bluetooth_enabled.eq(false),
                paired_devices::bluetooth_address.eq(Option::<String>::None),
                paired_devices::bluetooth_last_verified_at.eq(Option::<String>::None),
            ))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    }

    paired_devices::table
        .find(&input.peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_set_bluetooth_transport(
    db: State<AppDbConnection>,
    input: DeviceBluetoothTransportInput,
) -> Result<PairedDevice, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_set_bluetooth_transport_impl(&mut conn, input)
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
        crate::services::transport::ble::find_peer_address(
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

pub fn device_connection_transport_statuses_impl(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: String,
) -> Result<Vec<TransportStatus>, String> {
    let paired: PairedDevice = paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())?;
    let bluetooth_has_metadata = paired
        .bluetooth_address
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let bluetooth_os_paired = paired
        .bluetooth_address
        .as_deref()
        .and_then(normalize_bluetooth_address)
        .map(|address| bluetooth_address_is_os_paired(&address))
        .unwrap_or(false);

    // Phase 2 of ADR 0002: `session_kind` reports the *live* transport
    // (services::transport::TransportKind — TcpWs/Sim/Bluetooth/LoRa), a
    // finer-grained enum than this module's own Network/Bluetooth kind.
    // `Sim` maps to the Bluetooth row: it exists specifically to stand in
    // for Bluetooth's fallback role in tests/E2E (see
    // `transport::tests`'s own doc comment), so a live Sim session should
    // read the same way a live Bluetooth session would.
    let live_kind = state.session_kind(&peer_device_id);
    let network_connected = live_kind == Some(crate::services::transport::TransportKind::TcpWs);
    let bluetooth_connected = matches!(
        live_kind,
        Some(crate::services::transport::TransportKind::Bluetooth)
            | Some(crate::services::transport::TransportKind::Sim)
    );

    Ok(build_transport_statuses(
        state.network_peer_available(&peer_device_id),
        paired.bluetooth_enabled,
        bluetooth_has_metadata,
        bluetooth_os_paired,
        network_connected,
        bluetooth_connected,
    ))
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_transport_statuses(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
) -> Result<Vec<TransportStatus>, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_transport_statuses_impl(&mut conn, &state, peer_device_id)
}

/// Every paired peer eligible for a Bluetooth dial attempt right now:
/// Bluetooth-enabled, with a stored address, and currently OS-paired. Used
/// by `transport::ble::spawn_dial_loop` — unlike `tcp_ws`/`sim` there is no
/// presence worker or static port list to draw candidates from, so this
/// queries `paired_devices` directly, the same source
/// `device_connection_transport_statuses` already checks per-peer.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn bluetooth_dial_candidates(conn: &mut SqliteConnection) -> Vec<(String, String)> {
    let paired: Vec<PairedDevice> = paired_devices::table
        .filter(paired_devices::bluetooth_enabled.eq(true))
        .select(PairedDevice::as_select())
        .load(&mut *conn)
        .unwrap_or_default();

    paired
        .into_iter()
        .filter_map(|device| {
            let address = device.bluetooth_address.as_deref().and_then(normalize_bluetooth_address)?;
            bluetooth_address_is_os_paired(&address).then_some((device.peer_device_id, address))
        })
        .collect()
}

pub fn device_connection_session_transport_impl(
    state: &DeviceConnectionState,
    peer_device_id: String,
) -> Option<TransportKind> {
    state.session_kind(&peer_device_id)
}

/// Which transport (if any) the currently claimed session with a peer is
/// using. Debug/test surface proving the sticky single-session invariant
/// end-to-end through the real app binary — see
/// `specs/e2e/actors/tests/peer-sync-over-sim.spec.ts`.
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn device_connection_session_transport(
    state: State<DeviceConnectionState>,
    peer_device_id: String,
) -> Option<TransportKind> {
    device_connection_session_transport_impl(&state, peer_device_id)
}

pub fn device_connection_unpair_impl(
    conn: &mut SqliteConnection,
    peer_device_id: String,
) -> Result<(), String> {
    diesel::delete(paired_devices::table.find(&peer_device_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db;

    /// `FINI_BLUETOOTH_PAIRED_ADDRESSES` is process-global; tests that set
    /// and clear it must not interleave with each other under the default
    /// parallel test runner.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn paired_device_input(enabled: bool, address: Option<&str>) -> DeviceBluetoothTransportInput {
        DeviceBluetoothTransportInput {
            peer_device_id: "peer-a".to_string(),
            enabled,
            bluetooth_address: address.map(ToString::to_string),
        }
    }

    fn test_conn() -> SqliteConnection {
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
        conn
    }

    /// ADR 0002 Phase 3: a Bluetooth address handed over as part of the
    /// pairing handshake itself is trusted immediately on row creation,
    /// unlike the Device settings toggle (`set_bluetooth_transport_impl`,
    /// tested below) which requires OS pairing -- a completed handshake with
    /// human code confirmation is already a stronger trust signal.
    #[test]
    fn save_paired_device_populates_bluetooth_fields_on_insert_when_address_given() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        let saved = device_connection_save_paired_device_impl(
            &mut conn,
            "peer-new".to_string(),
            "Peer New".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
        )
        .expect("save paired device");

        assert!(saved.bluetooth_enabled);
        assert_eq!(saved.bluetooth_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert!(saved.bluetooth_last_verified_at.is_some());
    }

    #[test]
    fn save_paired_device_ignores_bluetooth_address_on_update() {
        // `test_conn` already seeds "peer-a" with bluetooth disabled --
        // saving over an *existing* row must only touch display_name/
        // last_seen_at, matching "create the new paired_devices row" in the
        // ADR: an address arriving via a duplicate pairing pass must not
        // silently override whatever the settings-page toggle set.
        let mut conn = test_conn();

        let saved = device_connection_save_paired_device_impl(
            &mut conn,
            "peer-a".to_string(),
            "Peer A Renamed".to_string(),
            Some("11:22:33:44:55:66".to_string()),
        )
        .expect("save paired device");

        assert_eq!(saved.display_name, "Peer A Renamed");
        assert!(!saved.bluetooth_enabled);
        assert_eq!(saved.bluetooth_address, None);
    }

    #[test]
    fn enabling_bluetooth_transport_requires_os_paired_address() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");
        let mut conn = test_conn();

        let missing = device_connection_set_bluetooth_transport_impl(
            &mut conn,
            paired_device_input(true, None),
        )
        .expect_err("address is required");
        assert!(missing.contains("address is required"));

        let unpaired = device_connection_set_bluetooth_transport_impl(
            &mut conn,
            paired_device_input(true, Some("11:22:33:44:55:66")),
        )
        .expect_err("OS pairing is required");
        assert!(unpaired.contains("OS Bluetooth pairing is required"));

        let paired = device_connection_set_bluetooth_transport_impl(
            &mut conn,
            paired_device_input(true, Some("aa:bb:cc:dd:ee:ff")),
        )
        .expect("paired address should enable");
        assert!(paired.bluetooth_enabled);
        assert_eq!(
            paired.bluetooth_address.as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert!(paired.bluetooth_last_verified_at.is_some());
        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    #[test]
    fn disabling_bluetooth_transport_clears_reconnect_metadata() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");
        let mut conn = test_conn();
        device_connection_set_bluetooth_transport_impl(
            &mut conn,
            paired_device_input(true, Some("AA:BB:CC:DD:EE:FF")),
        )
        .expect("enable bluetooth");

        let disabled = device_connection_set_bluetooth_transport_impl(
            &mut conn,
            paired_device_input(false, None),
        )
        .expect("disable bluetooth");
        assert!(!disabled.bluetooth_enabled);
        assert_eq!(disabled.bluetooth_address, None);
        assert_eq!(disabled.bluetooth_last_verified_at, None);
        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }
}
