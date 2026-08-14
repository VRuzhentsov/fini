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
use crate::services::device_connection::{build_transport_statuses, TransportStatus, TransportStatusInputs};
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

/// `paired_devices.preferred_transport`'s stored form for a live/target
/// `crate::services::transport::TransportKind` -- ADR-0003 Phase 3. `Sim`
/// (test/E2E-only, stands in for Bluetooth) and `LoRa` (reserved, no
/// adapter implements it yet) both fold into "bluetooth": neither is ever
/// a real user-facing preference target of its own.
pub(crate) fn transport_kind_to_preference_string(
    kind: crate::services::transport::TransportKind,
) -> &'static str {
    match kind {
        crate::services::transport::TransportKind::TcpWs => "network",
        crate::services::transport::TransportKind::Sim
        | crate::services::transport::TransportKind::Bluetooth
        | crate::services::transport::TransportKind::LoRa => "bluetooth",
    }
}

/// Tri-state OS-bond check: `Some(true)`/`Some(false)` are *confirmed*
/// results (the query actually completed), `None` means the check itself
/// failed or timed out -- inconclusive, not evidence the bond doesn't
/// exist. Most callers (the settings toggle, the accepting gate's
/// `check_bluetooth_bond`) correctly want to fail closed on `None` too --
/// see `bluetooth_address_is_os_paired`, the simple-bool wrapper they use.
/// `persist_bluetooth_address_and_maybe_enable` is the one caller that
/// needs to tell the difference, so a transient `bluetoothctl`/D-Bus
/// hiccup can't destructively clear a still-valid bond just because one
/// query attempt didn't complete.
fn bluetooth_address_bond_check(address: &str) -> Option<bool> {
    if let Ok(allowed) = std::env::var("FINI_BLUETOOTH_PAIRED_ADDRESSES") {
        return Some(
            allowed
                .split(',')
                .filter_map(normalize_bluetooth_address)
                .any(|item| item == address),
        );
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
            let mut command = tokio::process::Command::new("flatpak-spawn");
            command.arg("--host").arg("bluetoothctl");
            command
        } else {
            tokio::process::Command::new("bluetoothctl")
        };
        command.arg("info").arg(address);
        return tauri::async_runtime::block_on(bluetoothctl_bond_status(command, Duration::from_secs(5)));
    }

    #[cfg(target_os = "android")]
    {
        // `BluetoothPairing.isBonded` (com.fini.app, not ble-gatt's
        // dev.blegatt bridge — OS bond status is a Fini pairing
        // precondition, not part of the reusable GATT transport surface,
        // mirroring the Linux branch above shelling out to `bluetoothctl`
        // directly rather than through `ble_gatt::backend::linux`) queries
        // `BluetoothAdapter.getBondedDevices()` via JNI, returning `Boolean?`
        // on the Kotlin side so a permission/adapter/bridge failure is
        // distinguishable from a confirmed "not bonded" -- the tri-state
        // JNI helper, not the plain-bool one every *other* Android call
        // site in this file uses.
        return crate::services::android_context::call_static_context_string_to_optional_bool(
            "com.fini.app.BluetoothPairing",
            "isBonded",
            address,
        );
    }

    #[allow(unreachable_code)]
    None
}

/// Runs `bluetoothctl info` and reports whether it *completed* (regardless
/// of exit code -- a fast "device not found" answer is just as much a real
/// result as a "Paired: yes" line, both cases this device's controller
/// definitely resolved *something*), separately from whether it never
/// finished at all (`None`: failed to spawn, or ran past `timeout`).
/// `kill_on_drop(true)`, matching `run_command_with_timeout`: a timeout
/// elapsing here actually terminates and reaps the subprocess.
async fn bluetoothctl_bond_status(mut command: tokio::process::Command, timeout: Duration) -> Option<bool> {
    command
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().ok()?;
    let output = tokio::time::timeout(timeout, child.wait_with_output()).await.ok()?.ok()?;
    // A non-zero exit isn't necessarily "device unknown" -- it's just as
    // likely BlueZ, D-Bus, or the controller itself being temporarily
    // unavailable, an operational failure with nothing to say about the
    // actual bond. Only a successful run's content is a real answer.
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8(output.stdout)
            .map(|stdout| stdout.lines().any(|line| line.trim() == "Paired: yes"))
            .unwrap_or(false),
    )
}

/// Kept synchronous on purpose -- unlike `local_bluetooth_address`, this is
/// called from too many contexts (a plain sync Tauri command, an already
/// `block_in_place`-wrapped DB check, a raw non-runtime OS thread) to
/// thread `async`/`.await` through every caller safely. Fails closed
/// (`false`) on an inconclusive check, same as a confirmed "not paired" --
/// appropriate for every caller of this simple-bool wrapper (they all want
/// to fail closed regardless of *why* the answer was negative), unlike
/// `persist_bluetooth_address_and_maybe_enable`, which calls
/// `bluetooth_address_bond_check` directly to preserve that distinction.
pub(crate) fn bluetooth_address_is_os_paired(address: &str) -> bool {
    bluetooth_address_bond_check(address).unwrap_or(false)
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
/// and instead is discovered by a peer's BLE scan (`transport::ble`'s
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

/// Best-effort, fire-and-forget kickoff of the actual OS-level Bluetooth
/// bond for `address`, belonging to the already-paired `peer_device_id`.
/// Never blocks the caller. Triggered right after a BLE-first pair reveals
/// an address that isn't bonded yet (`device_connection_save_paired_device_impl`),
/// so the OS pairing prompt appears immediately instead of leaving the user
/// to find system Bluetooth settings on their own -- otherwise two freshly
/// BLE-paired devices with no shared network have no path to a working
/// Bluetooth session at all, since both `bluetooth_dial_candidates` and the
/// accepting gate hard-require OS bonding regardless of `bluetooth_enabled`.
///
/// Bonding itself completes asynchronously (the user has to respond to a
/// system dialog), so this also re-checks afterward and flips
/// `bluetooth_enabled` on the moment it observes the bond succeed --
/// otherwise nothing would ever revisit this pair again outside the
/// Device settings toggle, leaving a successfully-bonded pair stuck
/// disabled until the user happened to open Settings and flip it by hand.
pub(crate) fn request_os_bond(address: &str, peer_device_id: &str, db_path: std::path::PathBuf) {
    #[cfg(target_os = "linux")]
    {
        // `bluetoothctl pair` blocks on the pairing agent interaction (can
        // take several seconds) and only returns once it has resolved one
        // way or another, so a single re-check right after it returns is
        // enough to catch success -- no separate poll loop needed on this
        // platform. Same Flatpak `flatpak-spawn --host` routing as the
        // other `bluetoothctl` call sites in this file.
        let address = address.to_string();
        let peer_device_id = peer_device_id.to_string();
        std::thread::spawn(move || {
            let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
                let mut command = tokio::process::Command::new("flatpak-spawn");
                command.arg("--host").arg("bluetoothctl");
                command
            } else {
                tokio::process::Command::new("bluetoothctl")
            };
            command.arg("pair").arg(&address);
            // `bluetoothctl pair` can hang indefinitely on an unresponsive
            // BlueZ/D-Bus or a pairing agent nobody answers -- bounded the
            // same way as every other `bluetoothctl` call site in this
            // file (`run_command_with_timeout`'s `kill_on_drop` actually
            // terminates and reaps the subprocess on timeout, unlike the
            // plain `Command::output()` this replaces, which would leave
            // this thread and the child process stuck forever). A
            // generous 60s, not the 5s used for a quick read-only query:
            // this one involves real interactive user confirmation on one
            // or both ends, which can legitimately take a while.
            let _ = tauri::async_runtime::block_on(run_command_with_timeout(
                command,
                Duration::from_secs(60),
            ));
            if bluetooth_address_is_os_paired(&address) {
                let mut conn = crate::services::db::open_db_at_path(&db_path);
                if let Err(err) =
                    persist_bluetooth_address_and_maybe_enable(&mut conn, &peer_device_id, &address)
                {
                    eprintln!("[device-sync] persist bluetooth address after OS bond confirmed failed: {err}");
                }
            }
        });
    }
    #[cfg(target_os = "android")]
    {
        crate::services::android_context::call_static_context_string_void(
            "com.fini.app.BluetoothPairing",
            "createBond",
            address,
        );
        // `createBond()` only *requests* bonding -- completion is signaled
        // asynchronously via a system broadcast this process doesn't
        // observe directly (no JNI BroadcastReceiver bridge exists), so
        // poll `isBonded` for a bounded window instead.
        let address = address.to_string();
        let peer_device_id = peer_device_id.to_string();
        tauri::async_runtime::spawn(async move {
            const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
            const MAX_ATTEMPTS: u32 = 40; // ~2 minutes
            for _ in 0..MAX_ATTEMPTS {
                tokio::time::sleep(POLL_INTERVAL).await;
                if bluetooth_address_is_os_paired(&address) {
                    let mut conn = crate::services::db::open_db_at_path(&db_path);
                    if let Err(err) = persist_bluetooth_address_and_maybe_enable(
                        &mut conn,
                        &peer_device_id,
                        &address,
                    ) {
                        eprintln!(
                            "[device-sync] persist bluetooth address after OS bond confirmed failed: {err}"
                        );
                    }
                    return;
                }
            }
        });
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (address, peer_device_id, db_path);
    }
}

/// Stores `address` as `peer_id`'s Bluetooth address, and additionally
/// enables Bluetooth for the pair if -- and only if -- `address` is
/// currently OS-bonded *on this machine*. Returns whether it was enabled,
/// or an error if the write itself failed (a caller must not treat a
/// rejected/failed write as a successful enable or discovery).
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
) -> Result<bool, String> {
    // Read once upfront: `enabled` for the inconclusive case's "is there
    // anything to protect" check below, and `disabled_by_user` to keep an
    // explicit opt-out (`device_connection_set_bluetooth_transport_impl`'s
    // disable branch) from being silently undone by this self-report/
    // discovery path re-confirming a bond that never actually stopped
    // existing at the OS level -- the user's *Fini-level* choice to not
    // use it is a separate question from whether the OS thinks it's
    // bonded.
    let (currently_enabled, disabled_by_user): (bool, bool) = paired_devices::table
        .find(peer_id)
        .select((paired_devices::bluetooth_enabled, paired_devices::bluetooth_disabled_by_user))
        .first(&mut *conn)
        .unwrap_or((false, false));

    if disabled_by_user {
        // `specs/device-connect/README.md`: "Disabling ... clears stored
        // Bluetooth reconnect metadata," and that must *stay* cleared no
        // matter what this self-report's bond check would otherwise
        // conclude -- confirmed bonded, confirmed not bonded, or
        // inconclusive all leave the row untouched here. Checked before
        // even running the bond check itself: there is nothing this
        // self-report could learn that should change a row the user has
        // explicitly opted out of. Re-enabling via the settings toggle is
        // what stores a fresh address again, deliberately as its own
        // distinct user action.
        return Ok(false);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    let bond_check = bluetooth_address_bond_check(address);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let bond_check = Some(false);

    match bond_check {
        Some(true) => {
            // `disabled_by_user` was only checked above, before
            // `bluetooth_address_bond_check` ran -- that check can take up
            // to its own several-second timeout, a window in which the
            // user can disable Bluetooth for this pair (clearing metadata
            // and setting the opt-out) before this write lands. Filtering
            // the update on `bluetooth_disabled_by_user = false` rechecks
            // it atomically at write time instead of trusting the stale
            // snapshot from above, so a disable that landed mid-check
            // can't get silently undone by this branch re-enabling.
            let rows_affected = diesel::update(
                paired_devices::table
                    .find(peer_id)
                    .filter(paired_devices::bluetooth_disabled_by_user.eq(false)),
            )
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some(address)),
                paired_devices::bluetooth_last_verified_at
                    .eq(Some(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string())),
            ))
            .execute(conn)
            .map_err(|e| e.to_string())?;
            Ok(rows_affected > 0)
        }
        Some(false) => {
            // *Confirmed* not OS-bonded, so it must not keep whatever
            // `bluetooth_enabled`/`bluetooth_last_verified_at` an *older*,
            // possibly different address earned -- otherwise the row
            // would claim "enabled, verified at T" while actually
            // pointing at an address that was never verified at all, and
            // dial attempts would silently use it. Clear enablement and
            // verification atomically with the address update so the row
            // is never in that inconsistent state.
            diesel::update(paired_devices::table.find(peer_id))
                .set((
                    paired_devices::bluetooth_address.eq(Some(address)),
                    paired_devices::bluetooth_enabled.eq(false),
                    paired_devices::bluetooth_last_verified_at.eq(Option::<String>::None),
                ))
                .execute(conn)
                .map_err(|e| e.to_string())?;
            Ok(false)
        }
        None => {
            // Inconclusive (the check itself failed or timed out, e.g. a
            // transient BlueZ/D-Bus hiccup) -- not evidence the bond
            // doesn't exist. Whether it's safe to still record `address`
            // depends on what's already there: a peer with no *currently
            // enabled* Bluetooth state has nothing to protect (a brand new
            // pair, or one this same function already confirmed disabled),
            // so recording the self-reported address is harmless and
            // useful for the next check to target. But a peer that's
            // currently enabled has a previously-*verified* address/
            // timestamp on record -- overwriting just `bluetooth_address`
            // while leaving `bluetooth_enabled`/`bluetooth_last_verified_at`
            // at their old values would claim "enabled, verified at T" for
            // a replacement address that was never actually checked, so
            // that case leaves the entire tuple untouched instead.
            if !currently_enabled {
                diesel::update(paired_devices::table.find(peer_id))
                    .set(paired_devices::bluetooth_address.eq(Some(address)))
                    .execute(conn)
                    .map_err(|e| e.to_string())?;
            }
            Ok(false)
        }
    }
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
            let mut link = crate::services::transport::ble::dial(address).await?;
            crate::services::transport::send_frame(link.as_mut(), &msg).await
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
    // One toggle, both transports (ADR 0002 Phase 3): entering add-mode
    // makes this device discoverable over Bluetooth too, not just the
    // existing mDNS beacon.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::transport::ble::set_add_mode(true);
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
    via_bluetooth: bool,
    db_path: std::path::PathBuf,
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
    }

    // ADR 0002 Phase 3: a Bluetooth address handed over as part of the
    // pairing handshake itself (either observed directly on a
    // Bluetooth-carried completion, or self-reported by the peer) is
    // stored immediately -- but `bluetooth_enabled` still only flips on
    // if the address is actually OS-bonded (same gate Phase 1's
    // self-report already uses via `persist_bluetooth_address_and_maybe_enable`,
    // reused here). A completed pre-auth handshake proves reachability,
    // not bonding: `bluetooth_dial_candidates`/`check_bluetooth_bond`
    // both hard-require OS pairing regardless of this flag, so setting it
    // without a real bond would just be a lie the UI shows while the pair
    // can never actually establish a Bluetooth session.
    //
    // Runs for both branches above, not just a fresh insert: this
    // function is only ever called as the final step of a real,
    // human-confirmed pairing completion, never from an unrelated
    // background path -- an *existing* row here means an asymmetric
    // re-pair (the other side reset and paired again while this side kept
    // its old row), and that fresh handshake's Bluetooth details are just
    // as real as a brand-new pair's.
    if let Some(address) = bluetooth_address.as_deref().and_then(normalize_bluetooth_address) {
        // A fresh BLE-carried pairing completion is treated as an implicit
        // opt back *in*: an asymmetric re-pair (the other side reset and
        // paired again) can land on a row that still carries
        // `bluetooth_disabled_by_user = true` from a *previous* pairing
        // with this same peer_device_id, and
        // `persist_bluetooth_address_and_maybe_enable`'s opt-out guard
        // would otherwise silently ignore this completely fresh handshake
        // -- the UI would report pairing complete while this side
        // permanently rejects every real session. Completing a whole
        // BLE-first pairing (device discovery, code confirmation) is a
        // clear enough user action to count as re-opting in on its own,
        // unlike an ordinary network pairing that merely happens to carry
        // a self-reported address alongside it.
        if via_bluetooth {
            diesel::update(paired_devices::table.find(&peer_device_id))
                .set(paired_devices::bluetooth_disabled_by_user.eq(false))
                .execute(&mut *conn)
                .map_err(|e| e.to_string())?;
        }
        let enabled =
            persist_bluetooth_address_and_maybe_enable(&mut *conn, &peer_device_id, &address)?;
        // Only kick off the OS bond *request* (a system pairing prompt)
        // for the BLE-first flow this exists to unblock -- an ordinary
        // network pairing that happens to also carry a self-reported
        // Bluetooth address (both transports' details are always
        // exchanged regardless of which one carried completion) must not
        // surprise the user with a Bluetooth pairing dialog they never
        // asked for
        // (`docs/adr/0002-bluetooth-address-exchange-live-status-and-ble-pairing.md`).
        // Two freshly *BLE-paired* devices with no shared network do need
        // this, though: nothing else would ever prompt the user to
        // complete OS pairing for them.
        if !enabled && via_bluetooth {
            request_os_bond(&address, &peer_device_id, db_path.clone());
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
                // The user explicitly opted back in -- clears whatever a
                // previous explicit disable set, so self-reports are free
                // to auto-confirm/re-enable this pair again.
                paired_devices::bluetooth_disabled_by_user.eq(false),
            ))
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;
    } else {
        diesel::update(paired_devices::table.find(&input.peer_device_id))
            .set((
                paired_devices::bluetooth_enabled.eq(false),
                paired_devices::bluetooth_address.eq(Option::<String>::None),
                paired_devices::bluetooth_last_verified_at.eq(Option::<String>::None),
                // An explicit opt-out: must stick until the user explicitly
                // re-enables it above, not just until the next self-report
                // over a network session re-confirms the (still genuinely
                // OS-bonded) address -- see
                // `persist_bluetooth_address_and_maybe_enable`.
                paired_devices::bluetooth_disabled_by_user.eq(true),
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

/// ADR-0003 Phase 3: click either transport row to pin this pair to it.
/// Persists first (so it also governs *future* automatic reconnects, not
/// just this one), then -- if a session is currently live on some *other*
/// transport -- tells the peer first (while that session can still carry
/// the frame) and force-closes the local side of it. Deliberately
/// bypasses ADR-0001's sticky-handoff invariant for this one
/// user-initiated case; the next `space_sync_tick`'s dial loop picks the
/// peer back up on whichever transport `select_dial_order` now prefers.
/// `preferred: None` only clears the stored pin -- no session is
/// disturbed and no frame is sent, since there's no concrete target to
/// switch to or tell the peer about.
///
/// Notifying and force-closing are both skipped (leaving the live session
/// exactly as it is) when the peer's own negotiated `PROTOCOL_VERSION` is
/// below the one that introduced `PeerFrame::SwitchTransport` -- an older
/// peer can't decode the frame, and forcing a close it doesn't understand
/// would just have it reconnect over whatever transport *it* still knows
/// (typically Network), silently undoing the pin the moment it lands. The
/// preference is still persisted either way, so a future reconnect after
/// that peer upgrades (or this device's own dial loops, which consult the
/// stored preference directly) still honors it.
pub fn device_connection_set_preferred_transport_impl(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: String,
    preferred: Option<crate::services::transport::TransportKind>,
) -> Result<PairedDevice, String> {
    let existing: Option<PairedDevice> = paired_devices::table
        .find(&peer_device_id)
        .select(PairedDevice::as_select())
        .first(&mut *conn)
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.is_none() {
        return Err("paired device not found".to_string());
    }

    let now = utc_now();
    diesel::update(paired_devices::table.find(&peer_device_id))
        .set((
            paired_devices::preferred_transport
                .eq(preferred.map(transport_kind_to_preference_string)),
            paired_devices::preferred_transport_set_at.eq(Some(now.clone())),
        ))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;

    if let Some(kind) = preferred {
        // A live peer on an older build can't decode SwitchTransport at
        // all -- forcing the close anyway would just have it reconnect
        // over whatever it still understands, undoing this pin the moment
        // it lands (see this function's doc comment). `None` (no live
        // session) is *not* gated here: there's nothing to notify or close
        // either way, so it falls through harmlessly.
        let peer_understands_switch_transport = state
            .session_protocol_version(&peer_device_id)
            .is_none_or(|version| version >= crate::services::space_sync::types::PROTOCOL_VERSION);

        if peer_understands_switch_transport {
            // Enqueued into the same per-peer mailbox `request_session_close`
            // uses below, and mpsc preserves send order -- the peer receives
            // this frame before the local session (if any) actually closes.
            // Best-effort: `push_to_peer` returning false just means there was
            // no live session to carry it (nothing to notify), not an error.
            let _ = state.push_to_peer(
                &peer_device_id,
                PeerFrame::SwitchTransport { to: kind, requested_at: now },
            );

            if state.session_kind(&peer_device_id) != Some(kind) {
                state.request_session_close(&peer_device_id);
            }
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
pub fn device_connection_set_preferred_transport(
    db: State<AppDbConnection>,
    state: State<DeviceConnectionState>,
    peer_device_id: String,
    preferred: Option<crate::services::transport::TransportKind>,
) -> Result<PairedDevice, String> {
    let mut conn = db.0.lock().unwrap();
    device_connection_set_preferred_transport_impl(&mut conn, &state, peer_device_id, preferred)
}

/// Adopts a peer-proposed `PeerFrame::SwitchTransport` if it wins the
/// last-writer-wins race against whatever preference (if any) this device
/// already has recorded for the peer -- a `requested_at` that isn't
/// strictly newer than what's already stored is ignored. Returns whether
/// it was adopted; the caller (`session::handle_inbound`) only force-closes
/// the local session when this returns `true`, so a stale/losing frame
/// changes nothing. RFC3339-with-fixed-width-UTC timestamps (this
/// codebase's `utc_now()` form throughout) compare correctly as plain
/// strings -- no need to parse them into a real `DateTime`.
pub fn adopt_peer_transport_preference(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    to: crate::services::transport::TransportKind,
    requested_at: &str,
) -> bool {
    let current_set_at: Option<String> = paired_devices::table
        .find(peer_device_id)
        .select(paired_devices::preferred_transport_set_at)
        .first(&mut *conn)
        .unwrap_or(None);

    if let Some(current) = &current_set_at {
        if current.as_str() >= requested_at {
            return false;
        }
    }

    let preference = transport_kind_to_preference_string(to);
    let _ = diesel::update(paired_devices::table.find(peer_device_id))
        .set((
            paired_devices::preferred_transport.eq(Some(preference)),
            paired_devices::preferred_transport_set_at.eq(Some(requested_at)),
        ))
        .execute(&mut *conn);
    true
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

    Ok(build_transport_statuses(TransportStatusInputs {
        network_present: state.network_peer_available(&peer_device_id),
        network_reliable: state.network_effectively_available(&peer_device_id),
        bluetooth_enabled: paired.bluetooth_enabled,
        bluetooth_has_metadata,
        bluetooth_os_paired,
        bluetooth_reliable: state.bluetooth_effectively_reliable(&peer_device_id),
        network_connected,
        bluetooth_connected,
    }))
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

/// ADR-0003 Phase 3: this peer's manually-pinned transport preference, if
/// any -- `"network"`/`"bluetooth"` (`transport_kind_to_preference_string`'s
/// stored form), or `None` for pure automatic selection. Used by each
/// transport's own dial-gating logic (`tcp_ws::should_dial_peer`,
/// `ble::dial_with_backoff`, `sim::spawn_fallback_dial_loop`) to override
/// the default network-first order when the user has explicitly pinned the
/// other one. A missing/unpaired row reads the same as no preference,
/// matching every other `unwrap_or_default`-style read in this module.
pub fn peer_transport_preference(conn: &mut SqliteConnection, peer_id: &str) -> Option<String> {
    paired_devices::table
        .find(peer_id)
        .select(paired_devices::preferred_transport)
        .first::<Option<String>>(&mut *conn)
        .unwrap_or_default()
}

/// Like `peer_transport_preference`, but also returns the `requested_at`
/// it was set with -- `None` unless *both* columns are populated. Used by
/// `space_sync::session::run_session`'s own relay-on-establish check (see
/// its doc comment) to reconstruct a faithful `PeerFrame::SwitchTransport`
/// using the pin's original timestamp, not "now" -- the receiving peer's
/// last-writer-wins comparison depends on this being the real value.
pub fn peer_transport_preference_with_timestamp(
    conn: &mut SqliteConnection,
    peer_id: &str,
) -> Option<(String, String)> {
    let row: (Option<String>, Option<String>) = paired_devices::table
        .find(peer_id)
        .select((
            paired_devices::preferred_transport,
            paired_devices::preferred_transport_set_at,
        ))
        .first(&mut *conn)
        .ok()?;
    match row {
        (Some(preference), Some(requested_at)) => Some((preference, requested_at)),
        _ => None,
    }
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

/// `FINI_BLUETOOTH_PAIRED_ADDRESSES` is process-global; tests that set and
/// clear it must not interleave with each other under the default
/// parallel test runner. `pub(crate)` (not nested inside `mod tests`
/// below) and shared with `transport::tests`, which independently sets/
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

    /// Regression test for the P2 review finding: a bond check that never
    /// completes (timeout, here simulated with `sleep`) must report
    /// `None` -- inconclusive -- not `Some(false)`. Conflating the two is
    /// exactly what let a transient `bluetoothctl`/D-Bus hiccup
    /// destructively clear `bluetooth_enabled` for a peer whose bond very
    /// likely still exists.
    #[tokio::test]
    async fn bluetoothctl_bond_status_reports_none_when_the_check_times_out() {
        let mut command = tokio::process::Command::new("sleep");
        command.arg("30");
        let result = bluetoothctl_bond_status(command, Duration::from_millis(200)).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn bluetoothctl_bond_status_reports_some_true_when_paired() {
        let mut command = tokio::process::Command::new("printf");
        command.arg("Paired: yes\n");
        let result = bluetoothctl_bond_status(command, Duration::from_secs(5)).await;
        assert_eq!(result, Some(true));
    }

    /// A completed run that simply doesn't report a bond (a genuinely
    /// unknown/never-paired address, or a "not found" answer) is a real,
    /// confirmed result -- `Some(false)`, not `None` -- since the command
    /// resolved *something* within its deadline, unlike the timeout case
    /// above.
    #[tokio::test]
    async fn bluetoothctl_bond_status_reports_some_false_when_not_paired() {
        let mut command = tokio::process::Command::new("printf");
        command.arg("Paired: no\n");
        let result = bluetoothctl_bond_status(command, Duration::from_secs(5)).await;
        assert_eq!(result, Some(false));
    }

    /// Regression test for the P2 review finding: a non-zero exit isn't
    /// evidence the device is unpaired -- it's just as likely BlueZ,
    /// D-Bus, or the controller being temporarily unavailable, an
    /// operational failure this must report as inconclusive rather than
    /// a confirmed "not bonded."
    #[tokio::test]
    async fn bluetoothctl_bond_status_reports_none_on_a_nonzero_exit() {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg("exit 1");
        let result = bluetoothctl_bond_status(command, Duration::from_secs(5)).await;
        assert_eq!(result, None);
    }

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

    /// Regression test for the P1 review finding: an explicit disable via
    /// the settings toggle must stick even when a later self-report over
    /// an authenticated session re-confirms the address is genuinely
    /// still OS-bonded -- the user's Fini-level opt-out is a separate
    /// question from OS bond status, and `persist_bluetooth_address_and_maybe_enable`
    /// must not silently re-enable behind their back just because the
    /// bond never actually went away.
    #[test]
    fn persist_bluetooth_address_does_not_reenable_after_an_explicit_disable() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let mut conn = test_conn();
        diesel::update(paired_devices::table.find("peer-a"))
            .set((
                paired_devices::bluetooth_enabled.eq(false),
                paired_devices::bluetooth_address.eq(Option::<String>::None),
                paired_devices::bluetooth_last_verified_at.eq(Option::<String>::None),
                paired_devices::bluetooth_disabled_by_user.eq(true),
            ))
            .execute(&mut conn)
            .expect("seed an explicitly-disabled pair");

        // `persist_bluetooth_address_and_maybe_enable` itself doesn't
        // normalize -- callers do that first (see
        // `device_connection_save_paired_device_impl`/the inbound
        // self-report handler) -- so this passes an already-normalized
        // address, matching the real contract.
        let enabled =
            persist_bluetooth_address_and_maybe_enable(&mut conn, "peer-a", "AA:BB:CC:DD:EE:FF")
                .expect("persist bluetooth address");
        assert!(!enabled, "must not report enabled despite a confirmed bond");

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert!(
            !row.bluetooth_enabled,
            "an explicit disable must survive a self-report confirming the bond still exists"
        );
        assert_eq!(
            row.bluetooth_address, None,
            "specs/device-connect/README.md: disabling clears stored Bluetooth reconnect \
             metadata, and it must *stay* cleared -- not get quietly repopulated by the next \
             self-report"
        );
        assert!(row.bluetooth_last_verified_at.is_none());

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// Regression test for the P2 review finding: the disabled-by-user
    /// guard originally only covered the `Some(true)` (confirmed bonded)
    /// branch, so a self-report while the bond was confirmed *absent* or
    /// the check was inconclusive still repopulated `bluetooth_address`.
    /// The guard now runs before the bond check even executes, so this
    /// covers both remaining cases: a confirmed-not-paired address (no
    /// matching `FINI_BLUETOOTH_PAIRED_ADDRESSES` entry) leaves an
    /// explicitly-disabled peer's row untouched too.
    #[test]
    fn persist_bluetooth_address_ignores_a_confirmed_not_paired_result_after_an_explicit_disable() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

        let mut conn = test_conn();
        diesel::update(paired_devices::table.find("peer-a"))
            .set((
                paired_devices::bluetooth_enabled.eq(false),
                paired_devices::bluetooth_address.eq(Option::<String>::None),
                paired_devices::bluetooth_last_verified_at.eq(Option::<String>::None),
                paired_devices::bluetooth_disabled_by_user.eq(true),
            ))
            .execute(&mut conn)
            .expect("seed an explicitly-disabled pair");

        let enabled =
            persist_bluetooth_address_and_maybe_enable(&mut conn, "peer-a", "AA:BB:CC:DD:EE:FF")
                .expect("persist bluetooth address");
        assert!(!enabled);

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert_eq!(
            row.bluetooth_address, None,
            "a confirmed-not-paired result must not repopulate a disabled peer's address either"
        );
        assert!(!row.bluetooth_enabled);
        assert!(row.bluetooth_last_verified_at.is_none());

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// Regression test for the P1 review finding on the migration itself:
    /// `bluetooth_enabled` (migration 19) and `bluetooth_disabled_by_user`
    /// (this migration) ship together, unreleased -- no released build
    /// ever exposed a way to explicitly disable Bluetooth, so a
    /// pre-existing `bluetooth_enabled = 0` row is simply "never touched,"
    /// not "explicitly disabled." An earlier version of this migration
    /// backfilled such rows to `disabled_by_user = true`, which would have
    /// opted every existing pair out of the new automatic exchange flow on
    /// first upgrade. Reverts and re-applies just this migration to
    /// exercise the real SQL, not a hand-rolled equivalent.
    #[test]
    fn migration_does_not_backfill_disabled_by_user_for_preexisting_not_enabled_rows() {
        use diesel_migrations::MigrationHarness;

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        conn.revert_last_migration(db::MIGRATIONS)
            .expect("revert the bluetooth_disabled_by_user migration");

        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-legacy-untouched"),
                paired_devices::display_name.eq("Peer Legacy"),
                paired_devices::paired_at.eq("2026-01-01T00:00:00Z"),
                paired_devices::pair_state.eq("paired"),
                paired_devices::bluetooth_enabled.eq(false),
            ))
            .execute(&mut conn)
            .expect("seed a pre-existing not-enabled row, as if from before this migration");

        conn.run_pending_migrations(db::MIGRATIONS)
            .expect("reapply the bluetooth_disabled_by_user migration");

        let disabled_by_user: bool = paired_devices::table
            .find("peer-legacy-untouched")
            .select(paired_devices::bluetooth_disabled_by_user)
            .first(&mut conn)
            .expect("load the migrated row");
        assert!(
            !disabled_by_user,
            "a pre-existing not-enabled row must default to not-opted-out, since no released \
             build could have explicitly disabled it"
        );
    }

    /// Regression test for the same P1 finding: explicitly re-enabling via
    /// the settings toggle must clear the opt-out flag, so a *subsequent*
    /// self-report is free to auto-confirm/re-enable normally again.
    #[test]
    fn set_bluetooth_transport_clears_disabled_by_user_flag_when_re_enabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let mut conn = test_conn();
        diesel::update(paired_devices::table.find("peer-a"))
            .set(paired_devices::bluetooth_disabled_by_user.eq(true))
            .execute(&mut conn)
            .expect("seed an explicitly-disabled pair");

        device_connection_set_bluetooth_transport_impl(
            &mut conn,
            paired_device_input(true, Some("aa:bb:cc:dd:ee:ff")),
        )
        .expect("re-enable bluetooth transport");

        // Now a self-report must be free to keep it enabled/refresh
        // verification, since the explicit opt-out was cleared above.
        // (Already-normalized address, matching this lower-level
        // function's real contract -- see the sibling test's comment.)
        let enabled =
            persist_bluetooth_address_and_maybe_enable(&mut conn, "peer-a", "AA:BB:CC:DD:EE:FF")
                .expect("persist bluetooth address");
        assert!(enabled);

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// Regression test for the P2 review finding: an asymmetric BLE
    /// re-pair (the other side reset and paired again) can land on an
    /// *existing* row that still carries `bluetooth_disabled_by_user =
    /// true` from a previous pairing with this same peer_device_id.
    /// Without clearing it, `persist_bluetooth_address_and_maybe_enable`'s
    /// opt-out guard would silently ignore this completely fresh
    /// handshake -- the UI reports pairing complete, but this side
    /// permanently rejects every real session. Completing a whole
    /// BLE-first pairing counts as an implicit re-opt-in on its own.
    #[test]
    fn save_paired_device_clears_a_stale_opt_out_on_a_fresh_ble_carried_pairing() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let mut conn = test_conn();
        diesel::update(paired_devices::table.find("peer-a"))
            .set(paired_devices::bluetooth_disabled_by_user.eq(true))
            .execute(&mut conn)
            .expect("seed a stale opt-out from a previous pairing with this peer_device_id");

        let saved = device_connection_save_paired_device_impl(
            &mut conn,
            "peer-a".to_string(),
            "Peer A".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
            true, // via_bluetooth
            std::path::PathBuf::from("/nonexistent"), // never touched: enabled, no bond request
        )
        .expect("save paired device");

        assert!(
            saved.bluetooth_enabled,
            "a fresh BLE-carried pairing must not be silently ignored by a stale opt-out"
        );
        assert_eq!(saved.bluetooth_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// ADR 0002 Phase 3: a Bluetooth address handed over as part of the
    /// pairing handshake itself is stored on row creation, and enabled if
    /// it's also OS-bonded -- same gate Phase 1's self-report already uses
    /// (`persist_bluetooth_address_and_maybe_enable`). A completed pre-auth
    /// handshake proves reachability, not bonding: enabling without a real
    /// bond would be a dead end, since `bluetooth_dial_candidates` and the
    /// accepting gate both hard-require OS pairing regardless of this flag.
    #[test]
    fn save_paired_device_enables_bluetooth_on_insert_when_address_is_os_paired() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        let saved = device_connection_save_paired_device_impl(
            &mut conn,
            "peer-new".to_string(),
            "Peer New".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
            true,
            db_path.clone(),
        )
        .expect("save paired device");

        assert!(saved.bluetooth_enabled);
        assert_eq!(saved.bluetooth_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert!(saved.bluetooth_last_verified_at.is_some());

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    #[test]
    fn save_paired_device_stores_but_does_not_enable_bluetooth_on_insert_when_not_os_paired() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = db::open_db_at_path(&db_path);
        std::mem::forget(dir);

        let saved = device_connection_save_paired_device_impl(
            &mut conn,
            "peer-new-unbonded".to_string(),
            "Peer New Unbonded".to_string(),
            Some("aa:bb:cc:dd:ee:ff".to_string()),
            true,
            db_path.clone(),
        )
        .expect("save paired device");

        assert!(!saved.bluetooth_enabled);
        assert_eq!(saved.bluetooth_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert!(saved.bluetooth_last_verified_at.is_none());
    }

    /// Regression test for the P2 review finding: before this fix, an
    /// unbonded address update left `bluetooth_enabled`/
    /// `bluetooth_last_verified_at` untouched, so the row could end up
    /// claiming "enabled, verified" while actually pointing at an address
    /// that was never verified at all -- silently pointing dial attempts at
    /// an address that can't work while the metadata still looked healthy.
    #[test]
    fn persist_bluetooth_address_clears_enablement_when_the_new_address_is_not_os_paired() {
        let _guard = ENV_LOCK.lock().unwrap();
        // A *confirmed* not-paired result, not merely absent from the
        // allow-list of one: this must be `Some(false)`, not `None` --
        // `remove_var` alone would fall through to the real `bluetoothctl`
        // check, whose result for a nonexistent address depends on
        // whatever this machine's actual Bluetooth stack happens to
        // report (often now `None`/inconclusive, since a genuinely
        // unknown device typically makes `bluetoothctl info` exit
        // non-zero) -- not deterministic enough for this test's purpose.
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

        let mut conn = test_conn();
        diesel::update(paired_devices::table.find("peer-a"))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some("AA:BB:CC:DD:EE:FF")),
                paired_devices::bluetooth_last_verified_at.eq(Some("2026-01-01T00:00:00Z")),
            ))
            .execute(&mut conn)
            .expect("seed a previously-verified bluetooth address");

        let still_paired = persist_bluetooth_address_and_maybe_enable(
            &mut conn,
            "peer-a",
            "11:22:33:44:55:66",
        )
        .expect("persist bluetooth address");
        assert!(!still_paired);

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert_eq!(row.bluetooth_address.as_deref(), Some("11:22:33:44:55:66"));
        assert!(
            !row.bluetooth_enabled,
            "must not keep claiming enabled for an address that was never verified"
        );
        assert!(
            row.bluetooth_last_verified_at.is_none(),
            "stale verification timestamp must not survive an unbonded address update"
        );

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    }

    /// Regression test for the P2 review finding: this function is only
    /// ever called as the final step of a real pairing completion, never
    /// from an unrelated background path, so an *existing* row here means
    /// an asymmetric re-pair (the other side reset and paired again while
    /// this side kept its old row) -- that fresh handshake's Bluetooth
    /// details must not be silently dropped just because the row already
    /// existed.
    #[test]
    fn save_paired_device_refreshes_bluetooth_metadata_on_update_too() {
        let _guard = ENV_LOCK.lock().unwrap();
        // A *confirmed* not-paired result (see the sibling test's comment
        // above for why `remove_var` alone isn't deterministic enough).
        std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

        // `test_conn` seeds "peer-a" with no Bluetooth metadata at all, as
        // if from a stale prior pairing.
        let mut conn = test_conn();

        let saved = device_connection_save_paired_device_impl(
            &mut conn,
            "peer-a".to_string(),
            "Peer A Renamed".to_string(),
            Some("11:22:33:44:55:66".to_string()),
            false,
            // Never touched: not OS-paired, and via_bluetooth is false.
            std::path::PathBuf::from("/nonexistent"),
        )
        .expect("save paired device");

        assert_eq!(saved.display_name, "Peer A Renamed");
        assert_eq!(saved.bluetooth_address.as_deref(), Some("11:22:33:44:55:66"));

        std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
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

    /// ADR-0003 Phase 3's last-writer-wins race resolution: a peer-proposed
    /// preference with no existing preference recorded locally always wins.
    #[test]
    fn adopt_peer_transport_preference_adopts_when_nothing_is_recorded_yet() {
        let mut conn = test_conn();

        let adopted =
            adopt_peer_transport_preference(&mut conn, "peer-a", TransportKind::Bluetooth, "2026-04-07T00:00:01Z");
        assert!(adopted);

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert_eq!(row.preferred_transport.as_deref(), Some("bluetooth"));
        assert_eq!(row.preferred_transport_set_at.as_deref(), Some("2026-04-07T00:00:01Z"));
    }

    /// A strictly newer `requested_at` than what's already recorded wins
    /// and overwrites both the preference and its timestamp.
    #[test]
    fn adopt_peer_transport_preference_adopts_a_strictly_newer_timestamp() {
        let mut conn = test_conn();
        assert!(adopt_peer_transport_preference(
            &mut conn,
            "peer-a",
            TransportKind::TcpWs,
            "2026-04-07T00:00:01Z"
        ));

        let adopted = adopt_peer_transport_preference(
            &mut conn,
            "peer-a",
            TransportKind::Bluetooth,
            "2026-04-07T00:00:02Z",
        );
        assert!(adopted, "a newer requested_at must win");

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert_eq!(row.preferred_transport.as_deref(), Some("bluetooth"));
        assert_eq!(row.preferred_transport_set_at.as_deref(), Some("2026-04-07T00:00:02Z"));
    }

    /// An older `requested_at` than what's already recorded loses and must
    /// leave the existing preference untouched.
    #[test]
    fn adopt_peer_transport_preference_rejects_an_older_timestamp() {
        let mut conn = test_conn();
        assert!(adopt_peer_transport_preference(
            &mut conn,
            "peer-a",
            TransportKind::TcpWs,
            "2026-04-07T00:00:02Z"
        ));

        let adopted = adopt_peer_transport_preference(
            &mut conn,
            "peer-a",
            TransportKind::Bluetooth,
            "2026-04-07T00:00:01Z",
        );
        assert!(!adopted, "an older requested_at must lose");

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert_eq!(row.preferred_transport.as_deref(), Some("network"));
        assert_eq!(row.preferred_transport_set_at.as_deref(), Some("2026-04-07T00:00:02Z"));
    }

    /// A tied `requested_at` also loses -- the comparison is `>=`, not `>`,
    /// so the side that already recorded a value keeps it rather than the
    /// two racing writers flapping the preference back and forth forever.
    #[test]
    fn adopt_peer_transport_preference_rejects_an_equal_timestamp() {
        let mut conn = test_conn();
        assert!(adopt_peer_transport_preference(
            &mut conn,
            "peer-a",
            TransportKind::TcpWs,
            "2026-04-07T00:00:01Z"
        ));

        let adopted = adopt_peer_transport_preference(
            &mut conn,
            "peer-a",
            TransportKind::Bluetooth,
            "2026-04-07T00:00:01Z",
        );
        assert!(!adopted, "a tied requested_at must lose");

        let row: PairedDevice = paired_devices::table
            .find("peer-a")
            .select(PairedDevice::as_select())
            .first(&mut conn)
            .expect("load peer row");
        assert_eq!(row.preferred_transport.as_deref(), Some("network"));
    }
}
