//! The Bluetooth transport: BLE GATT `DataLink`s over `ble-gatt`'s datagram tier
//! (github.com/VRuzhentsov/ble-gatt).
//!
//! Two kinds of link share one GATT service (ADR-0009 D2, D3, D8):
//!
//! - **Sessions** (`Auth`, channel `Hello`) are iroh connections over
//!   `ble-gatt-iroh`, to the key pinned for the pair, which TLS proves
//!   (`dial_session`, `bluetooth_endpoint`).
//! - **Pre-pairing frames** (discovery, pair request/accept/complete) go over
//!   a plain `BleDataLink`: there is no pinned key to dial yet, so each frame
//!   carries the sender's key itself.
//!
//! An accepted channel is routed by its first datagram (`route_inbound`): a
//! QUIC Initial goes to the iroh transport, Fini's JSON to the gate.
//!
//! Linux (BlueZ via `ble_gatt::backend::linux`) and Android (the backend
//! `tauri-plugin-ble-gatt` builds; Fini uses it from Rust, not through Tauri
//! IPC). See `backend` below for why construction is deferred, and
//! `start_peripheral_once`/its caller in `sync::commands` for why the
//! peripheral role isn't spawned from `.setup()` on Android the way it is on
//! Linux.
//!
//! Plays the same role `channel::loopback` plays for tests/E2E, but for real.
//! ADR-0003 revision: dials/accepts unconditionally, independent of
//! Network's own state -- both transports stay connected to a paired peer
//! at once, with `preferred_transport` only deciding which one is primary
//! (see `pairing::DeviceConnectionState::recompute_primary_locked`),
//! not whether Bluetooth connects at all. Unlike `network` (backed by the
//! mDNS/UDP presence worker) and `sim` (statically configured ports), there
//! is no discovery step here — candidates come from stored per-peer
//! Bluetooth metadata (`paired_devices.bluetooth_address`, gated on
//! `bluetooth_enabled` and a live OS-pairing check), exactly what
//! `pairing::commands::bluetooth_address_is_os_paired` already
//! checks for the enable command.

#[cfg(any(feature = "ui-plane", test))]
use std::collections::HashSet;
#[cfg(any(feature = "ui-plane", test))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use ble_gatt::backend::linux::LinuxBackend;
use ble_gatt::datagram::{self, DatagramChannel, DatagramConfig};
use ble_gatt::{Backend, CharacteristicUuid, PeerAddress, ServiceUuid};
#[cfg(target_os = "linux")]
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::{
    bluetooth_dial_candidates, note_observed_bluetooth_address as store_observed_bluetooth_address,
    DeviceConnectionState,
};
use crate::services::communication::sync::session;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::sync::types::PeerFrame;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::channel::{recv_frame, send_frame};
use crate::services::communication::channel::iroh_link::{IrohDataLink, ALPN};
use crate::services::communication::channel::{BoxDialFuture, DataLink, Transport, ChannelKind};

mod adapter;
mod advertiser;
mod data_link;
#[cfg(any(feature = "ui-plane", test))]
mod discovery;
mod endpoint;
#[cfg(any(feature = "ui-plane", test))]
mod peer_directory;
mod presence;
mod radio_arbiter;

use adapter::*;
#[cfg(any(feature = "ui-plane", test))]
pub use adapter::{is_bluetooth_adapter_unavailable, probe_adapter_available};
#[cfg(test)]
pub use adapter::pin_adapter_reachable_on_this_thread;
pub use data_link::*;
#[cfg(any(feature = "ui-plane", test))]
pub use discovery::*;
pub use endpoint::*;

pub use advertiser::Advertiser;
#[cfg(any(feature = "ui-plane", test))]
pub use peer_directory::PeerDirectory;
pub use presence::Presence;
pub use radio_arbiter::RadioArbiter;
use radio_arbiter::Purpose;

/// Fini's own GATT service/characteristic for the datagram tier. Fixed, not
/// user-configurable: both sync peers must advertise/expect the same UUIDs
/// to find each other's service. Distinct from any third-party device's own
/// UUIDs — this is Fini-to-Fini only, the app-to-app case `ble-gatt`'s
/// ADR-0003 describes.
const FINI_BLE_SERVICE_UUID: &str = "b1e6a000-f101-4000-8000-00805f9b34fb";
const FINI_BLE_CHARACTERISTIC_UUID: &str = "b1e6a001-f101-4000-8000-00805f9b34fb";

/// Fini's service and characteristic, for dialling and scanning. What this
/// device advertises adds its identity: `Advertiser::advertised_config`.
fn datagram_config() -> DatagramConfig {
    DatagramConfig::new(
        ServiceUuid(Uuid::parse_str(FINI_BLE_SERVICE_UUID).expect("valid UUID literal")),
        CharacteristicUuid(Uuid::parse_str(FINI_BLE_CHARACTERISTIC_UUID).expect("valid UUID literal")),
    )
}

/// Four bytes of FNV-1a over the `device_id`.
///
/// FNV-1a specifically, and **not** `std::collections::hash_map::DefaultHasher`:
/// that one's output is explicitly not guaranteed stable across Rust
/// releases, so two peers built with different toolchains would compute
/// different fingerprints for the same device and never recognise each
/// other. FNV-1a is a fixed, published algorithm, so the value is stable
/// for as long as the `device_id` is.
///
/// Not a security boundary and not meant to be one: `Auth` is
/// (`specs/device-connect/README.md`), and this only decides which
/// advertiser is worth dialling. A collision costs one wasted dial that
/// `Auth` then rejects.
///
/// ADR-0006 records the privacy limit this carries: a stable value
/// broadcast continuously is trackable, which is why the rotating,
/// secret-derived form is the intended successor once pairing establishes
/// key material (issue #162).
fn fingerprint_of(device_id: &str) -> [u8; FINGERPRINT_LEN] {
    const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in device_id.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash.to_be_bytes()
}

/// Reads the fingerprint out of a discovered peer's manufacturer data, or
/// `None` for an advertiser that carries none -- a peer running a build
/// from before ADR-0006's slice 2, which must stay dialable rather than
/// being filtered out for being old.
fn advertised_fingerprint(manufacturer_payload: Option<&[u8]>) -> Option<[u8; FINGERPRINT_LEN]> {
    let payload = manufacturer_payload?;
    payload.get(1..1 + FINGERPRINT_LEN)?.try_into().ok()
}

const FINGERPRINT_LEN: usize = 4;

/// `0xFFFF` is the Bluetooth SIG's own reserved value for "manufacturer
/// specific data" used for testing and non-market purposes — the
/// appropriate choice for a private, unregistered app like Fini rather
/// than picking an arbitrary value that could collide with a real vendor's
/// company ID some other nearby scanner is specifically watching for.
const FINI_MANUFACTURER_ID: u16 = 0xFFFF;
/// Bit 0 of the manufacturer payload's first byte: whether this device is
/// currently in add-mode.
///
/// The payload used to be exactly this one byte, present only while in
/// add-mode. ADR-0006's slice 2 made it a five-byte record — one flags
/// byte then four fingerprint bytes — advertised always, because the
/// fingerprint is how a scanner tells which peer it has found. Five bytes
/// still fits: legacy advertisements cap at 31 total and the 128-bit
/// service UUID plus this record spend about 26. See
/// `GattServiceSpec::manufacturer_data`'s own doc comment in ble-gatt.
#[cfg(any(feature = "ui-plane", test))]
const ADD_MODE_FLAG_BYTE: u8 = 0x01;

/// Starts the Bluetooth peripheral acceptor loop exactly once. Android-only:
/// on Linux `lib.rs` spawns `run_server` unconditionally from `.setup()`,
/// which is safe there since `LinuxBackend::new()` has no Android-context
/// ordering requirement. On Android that same eager spawn would race
/// `tao`'s own context bring-up (see `backend`), so the
/// first call instead comes from `space_sync_tick_impl` — a
/// `#[tauri::command]`, whose first real invocation can only happen once
/// the WebView/Activity has actually dispatched an IPC call, a strictly
/// later and safer point than anything obtainable from `.setup()` itself.
/// The running check is a cheap filter -- this runs from every tick, and
/// without it each one would spawn a task only for
/// `Advertiser::claim_peripheral_role` to turn it away. The claim in
/// `run_server` is the actual invariant; this just keeps the common path
/// quiet.
#[cfg(target_os = "android")]
pub fn start_peripheral_once(state: DeviceConnectionState, db_path: PathBuf) {
    if !state.bluetooth_advertiser.peripheral_running() {
        tauri::async_runtime::spawn(run_server(state, db_path));
    }
}

/// `Transport` implementation for the Bluetooth adapter — see the note on
/// `channel::tcp_ws::TcpWsTransport` for why production dial loops call
/// `dial()` directly rather than through this trait object.
#[allow(dead_code)]
pub struct BleTransport;

#[async_trait]
impl Transport for BleTransport {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn dial(&self, _peer_device_id: &str, addr: &str, _port: u16) -> BoxDialFuture {
        let addr = addr.to_string();
        Box::pin(async move { dial(&addr).await })
    }
}

/// Peripheral role: advertise Fini's BLE service and gate every accepted
/// central through the same transport-neutral session gate every other
/// adapter uses. No-op (logs and returns) when the local adapter can't do
/// peripheral mode, or isn't available at all — Bluetooth is always a
/// fallback, never a hard requirement to start the app. `ui-plane`/`test`
/// only, matching `tcp_ws::run_server`/`loopback::run_server` — `cli-plane` dials
/// out but does not run an inbound acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_server(state: DeviceConnectionState, db_path: PathBuf) {
    use futures_util::StreamExt;

    let advertiser = state.bluetooth_advertiser.clone();
    if !advertiser.claim_peripheral_role() {
        log::warn!("[transport][ble] peripheral acceptor already running; ignoring a second start");
        return;
    }

    // Retried with backoff, not returned-from-once: `lib.rs` spawns this
    // exactly once at startup, so an early failure here (adapter off,
    // BlueZ restarting, briefly unavailable) used to end the peripheral
    // role for the rest of the process's life even after `backend()`
    // itself became able to retry. If this device also has the higher
    // device id, the deterministic dial rule means it never dials out
    // either — Bluetooth would be unusable until restart. Looping means
    // enabling Bluetooth later (or the adapter coming back) can still
    // stand up the acceptor.
    let mut delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(60);

    let mut advertising_wanted = advertiser.watch_wanted();
    loop {
        // ADR-0008 D8: advertise only while there is someone this device
        // should be reachable by.
        while !*advertising_wanted.borrow_and_update() {
            if advertising_wanted.changed().await.is_err() {
                return;
            }
        }
        let backend = match backend().await {
            Ok(backend) => backend,
            Err(err) => {
                note_adapter_unreachable();
                log::warn!("[transport][ble] adapter unavailable, retrying in {delay:?}: {err}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
                continue;
            }
        };
        if !backend.capabilities().await.peripheral {
            log::warn!(
                "[transport][ble] adapter has no peripheral support; retrying in {delay:?} \
                 in case a different adapter becomes available"
            );
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(max_delay);
            continue;
        }
        // Subscribed *before* the config snapshot/`serve` call below, not
        // after: a `watch::Receiver` only misses changes that happen
        // strictly before it subscribes, so subscribing here closes the
        // window where a toggle lands while the advertisement (built from
        // the config snapshot `serve` takes) is still starting up -- a real
        // async operation, not instant. Subscribing afterward would let
        // that specific toggle go unseen (the receiver's baseline already
        // reflects the new value at subscribe time), leaving the device
        // advertising without the add-mode flag, undiscoverable, until
        // some *later* toggle happens to fire `changed()` again. Watched
        // (not merely read) so a toggle mid-serve interrupts the accept
        // loop below immediately, rather than only taking effect on
        // whatever later triggers a natural re-advertise -- see
        // `Advertiser::add_mode`'s doc comment for why this is a signal to
        // the running loop rather than a full task restart.
        let mut add_mode_rx = advertiser.watch_add_mode();
        let mut incoming = match datagram::serve(backend, &advertiser.advertised_config()).await {
            Ok(stream) => {
                // Advertising is a real use of the radio, so it clears a
                // previously-recorded failure just as a scan does.
                note_adapter_reachable();
                stream
            }
            Err(err) => {
                note_adapter_unreachable();
                log::warn!("[transport][ble] advertise failed, retrying in {delay:?}: {err}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
                continue;
            }
        };
        log::info!("[transport][ble] advertising, awaiting centrals");
        delay = Duration::from_secs(2);

        let mut restarting_for_add_mode_change = false;
        // BlueZ stops transmitting the advertisement once a central that
        // connected to it has gone, while still reporting the instance as
        // active (`ActiveInstances: 1`). Measured: the phone heard the
        // desktop's advertisement three times right after it was
        // registered, a phone-initiated hello then connected and left, and
        // the phone heard nothing for 4.5 minutes -- until the advertisement
        // was registered again. So the advertisement is re-registered when
        // the last accepted central is gone; doing it earlier would tear down
        // the live link.
        let live_centrals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (central_gone_tx, mut central_gone_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        loop {
            tokio::select! {
                Some(()) = central_gone_rx.recv() => {
                    // Linux only. Android keeps advertising, and registering
                    // the advertisement again gives the phone a new private
                    // address: the address the other device had just heard
                    // goes stale, and its dial hangs for 15 s before the
                    // search hears the new one.
                    if cfg!(target_os = "linux") && live_centrals.load(Ordering::SeqCst) == 0 {
                        log::info!("[transport][ble] last central gone; re-advertising");
                        restarting_for_add_mode_change = true;
                        break;
                    }
                }
                channel = incoming.next() => {
                    let Some(channel) = channel else { break; };
                    // Without this, a link that dies before it sends anything
                    // (lost at the GATT layer before the first read) would
                    // leave no trace that anything was accepted.
                    log::info!("[transport][ble] central connected: {}", channel.peer().0);
                    let state = state.clone();
                    let db_path = db_path.clone();
                    live_centrals.fetch_add(1, Ordering::SeqCst);
                    let live_centrals = live_centrals.clone();
                    let central_gone_tx = central_gone_tx.clone();
                    // The central counts as live until its channel closes:
                    // the gate finishing on a plain link, or the iroh
                    // transport closing the channel of a session.
                    tokio::spawn(async move {
                        // The device connected to us is the one dialling: the
                        // add-mode scan must not dial back while it is here.
                        // A second, crossing connection to the same device is
                        // what kept dropping both (initiator = central).
                        let _leg = state.bluetooth_radio.begin_pairing_leg();
                        if let Some(handled) = route_inbound(&state, db_path, channel).await {
                            let _ = handled.await;
                        }
                        live_centrals.fetch_sub(1, Ordering::SeqCst);
                        let _ = central_gone_tx.send(());
                    });
                }
                _ = add_mode_rx.changed() => {
                    log::info!("[transport][ble] add-mode changed; re-advertising");
                    restarting_for_add_mode_change = true;
                    break;
                }
                _ = advertising_wanted.changed() => {
                    if !*advertising_wanted.borrow() {
                        log::info!("[transport][ble] nothing to be reachable for; stopping advertising");
                        restarting_for_add_mode_change = true;
                        break;
                    }
                }
            }
        }
        if restarting_for_add_mode_change {
            // Ending `incoming` here (by falling through to the outer
            // loop's next `datagram::serve` call) is what actually stops
            // the old advertisement -- ble-gatt's backends tear down the
            // previous generation's GATT server/advertisement as part of
            // starting a new one (see BleGattBridge.startAdvertising's own
            // doc comment: "tear down any predecessor first").
            delay = Duration::from_secs(2);
            continue;
        }
        // The serve stream itself ended (e.g. the adapter dropped out from
        // under it) rather than a caller closing it or an add-mode change
        // — nothing else here ever drops the stream deliberately. Retry
        // rather than leaving the peripheral role dead.
        log::warn!("[transport][ble] serve stream ended unexpectedly; retrying in {delay:?}");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

/// The Bluetooth half of ADR-0008 D10/D12: presence, search and exchanges.
///
/// - **Presence**: a paired peer whose advertisement was heard within
///   `BLUETOOTH_CHANNEL_TIMEOUT` is present (green).
/// - **Search** is one coordinator (`search`, ADR-0008 D13) serving every
///   purpose with a single scan:
///   - status, while the Device page is open, keeps presence current;
///   - delivery, when there is work for a peer, searches up to
///     `SEARCH_WINDOW` and connects as soon as it is heard; a peer not found
///     is tried again after `DELIVERY_RETRY` steps, until it appears;
///   - setup, while the setup dialog is open, says hello to the peer.
/// - **Exchange**: connect, authenticate, push and acknowledge, then close
///   once idle (`session::run_session`).
///
/// Nothing here runs when there is no reason to: no work, no open page, no
/// setup -- the radio only advertises and listens for connections.
const SEARCH_WINDOW: Duration = Duration::from_secs(60);

/// Per-candidate cap for one dial + authentication inside a delivery search:
/// an advertiser that accepts the connection and never answers must not
/// hold the whole window.
const DIAL_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one status-search window listens while the Device page is open.
const STATUS_SEARCH_WINDOW: Duration = Duration::from_secs(20);

/// Start an exchange with this peer over Bluetooth unless one is running or
/// being attempted (ADR-0008 D10). Called when there is work for the peer.
pub fn start_exchange(state: &DeviceConnectionState, peer_id: &str) {
    if state.has_session_on(peer_id, ChannelKind::Bluetooth) || !state.bluetooth_presence.delivery_due(peer_id) {
        return;
    }
    // While a device is being added, the adapter belongs to that search.
    if state.bluetooth_advertiser.in_add_mode() {
        return;
    }
    if !state.bluetooth_radio.begin_exchange(peer_id) {
        return;
    }
    let state = state.clone();
    let peer_id = peer_id.to_string();
    tauri::async_runtime::spawn(async move {
        exchange_with(&state, &peer_id).await;
        state.bluetooth_radio.end_exchange(&peer_id);
    });
}

/// One delivery (ADR-0008 D8, D12): a present peer is one dial to where it
/// was last heard; otherwise a delivery search of up to `SEARCH_WINDOW`
/// connects the moment the peer is heard. Then authenticate and run the
/// exchange to its idle end.
async fn exchange_with(state: &DeviceConnectionState, peer_id: &str) {
    let db_path = state.db_path.clone();
    if !is_still_bluetooth_eligible(&db_path, peer_id) {
        return;
    }
    let deadline = tokio::time::Instant::now() + SEARCH_WINDOW;
    let mut last_error = None;

    if let Some(address) = state.bluetooth_presence.last_seen_address(peer_id) {
        let guard = state.bluetooth_radio.acquire_dial().await;
        match connect_and_auth(state, peer_id, &address).await {
            Ok((link, version)) => {
                drop(guard);
                return run_exchange(state, peer_id, link, version, &address).await;
            }
            // An exchange with the peer already runs (its own dial, or the
            // one that won a crossing): the work goes through that.
            Err(err) if session::refused_for_running_exchange(&err) => return,
            Err(err) => last_error = Some(err),
        }
    }

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let found = if remaining.is_zero() {
            None
        } else {
            state.bluetooth_radio.find(peer_id, Purpose::Delivery, remaining).await
        };
        let Some(found) = found else {
            match last_error {
                Some(err) => log::info!("[transport][ble] {peer_id} refused the exchange: {err}"),
                None => log::info!("[transport][ble] {peer_id} not heard within the search window"),
            }
            state.bluetooth_presence.note_delivery_missed(peer_id);
            return;
        };
        match connect_and_auth(state, peer_id, &found.address).await {
            Ok((link, version)) => {
                let address = found.address.clone();
                drop(found);
                return run_exchange(state, peer_id, link, version, &address).await;
            }
            Err(err) if session::refused_for_running_exchange(&err) => return,
            Err(err) => last_error = Some(err),
        }
    }
}

/// Dial `address` and authenticate as talking to `peer_id`, bounded so an
/// advertiser that accepts the connection and never answers cannot hold the
/// search. One real `connect()` was measured at ~28s.
async fn connect_and_auth(
    state: &DeviceConnectionState, peer_id: &str, address: &str,
) -> Result<(Box<dyn DataLink>, u32), String> {
    let attempt = tokio::time::timeout(DIAL_CANDIDATE_TIMEOUT, async {
        let mut link = dial_session(state, peer_id, address).await?;
        let version =
            session::perform_client_auth(link.as_mut(), &state.identity.device_id, peer_id).await?;
        Ok::<_, String>((link, version))
    })
    .await;
    match attempt {
        Ok(Ok(connected)) => Ok(connected),
        Ok(Err(err)) => {
            log::info!("[transport][ble] candidate {address} is not {peer_id}: {err}");
            if !session::refused_for_running_exchange(&err) {
                state.bluetooth_radio.note_dial_failed(address);
            }
            Err(err)
        }
        Err(_elapsed) => {
            log::info!("[transport][ble] candidate {address} did not finish connect+auth in time");
            state.bluetooth_radio.note_dial_failed(address);
            Err("connect+auth timed out".to_string())
        }
    }
}

async fn run_exchange(
    state: &DeviceConnectionState, peer_id: &str, link: Box<dyn DataLink>, version: u32, address: &str,
) {
    state.bluetooth_presence.forget_delivery_misses(peer_id);
    log::info!("[transport][ble] exchange with {peer_id} via {address}");
    let db_path = state.db_path.clone();
    // The switch could have been turned off during the search.
    if !is_still_bluetooth_eligible(&db_path, peer_id) {
        return;
    }
    // Diagnostics only: nothing dials a stored address.
    note_observed_bluetooth_address(&db_path, peer_id, address);
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    if state.try_claim_session(peer_id, ChannelKind::Bluetooth, tx) {
        session::run_session(
            link,
            rx,
            state.clone(),
            db_path,
            peer_id.to_string(),
            version,
            session::EXCHANGE_IDLE,
        )
        .await;
    }
}

/// Re-reads `paired_devices` for the current, live answer to "is this peer
/// still a valid Bluetooth dial target" — which since ADR-0006 means only
/// "is Bluetooth still enabled for this pair". `block_in_place` around the
/// blocking DB open, matching `sync::session::check_paired`'s existing
/// pattern for the same kind of call from inside an async loop.
fn is_still_bluetooth_eligible(db_path: &std::path::Path, peer_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        bluetooth_dial_candidates(&mut conn).iter().any(|candidate_id| candidate_id == peer_id)
    })
}

fn note_observed_bluetooth_address(db_path: &std::path::Path, peer_id: &str, address: &str) {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        store_observed_bluetooth_address(&mut conn, peer_id, address);
    })
}

/// Turn the status search on while the Device page is open, off when it
/// closes. Green is never computed in the background (ADR-0008 D12); the
/// search coordinator merges it with any delivery or setup search running.
#[cfg(any(feature = "ui-plane", test))]
pub fn set_status_search(state: &DeviceConnectionState, active: bool) {
    state.bluetooth_radio.set_status(active.then(|| state.db_path.clone()));
}

/// Recompute whether to advertise, from the stored channels and the setups
/// running now. Cheap; called whenever one of its inputs may have changed.
pub fn refresh_advertising(db_path: &std::path::Path, state: &DeviceConnectionState) {
    let any_channel_on = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        !crate::services::communication::pairing::channels::peers_with_channel_enabled(
            &mut conn,
            ChannelKind::Bluetooth,
        )
        .is_empty()
    });
    let advertiser = &state.bluetooth_advertiser;
    let wanted = any_channel_on
        || state.any_channel_setup(ChannelKind::Bluetooth)
        || advertiser.answering_after_setup()
        || advertiser.in_add_mode();
    advertiser.set_wanted(wanted);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fingerprint must be exactly FNV-1a, because both peers compute it
    /// independently from the same `device_id` and any divergence means they
    /// stop recognising each other -- a failure that would only appear across
    /// an app-version boundary and would look like a radio problem.
    ///
    /// Checked against the published algorithm spelled out here rather than
    /// against a captured constant: this catches a silent swap to a different
    /// hash (`DefaultHasher`, whose output is explicitly unstable across Rust
    /// releases, being the tempting one) without needing a magic number whose
    /// provenance a later reader cannot verify.
    #[test]
    fn fingerprint_is_fnv1a_over_the_device_id() {
        fn reference_fnv1a(input: &str) -> [u8; FINGERPRINT_LEN] {
            let mut hash: u32 = 2_166_136_261;
            for byte in input.as_bytes() {
                hash ^= u32::from(*byte);
                hash = hash.wrapping_mul(16_777_619);
            }
            hash.to_be_bytes()
        }

        for id in ["75700b2e-c970-482f-aa16-63ebdde0a91c", "peer-a", ""] {
            assert_eq!(fingerprint_of(id), reference_fnv1a(id), "mismatch for {id:?}");
        }
        assert_ne!(fingerprint_of("peer-a"), fingerprint_of("peer-b"));
    }

}
