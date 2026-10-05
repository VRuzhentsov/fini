//! The Bluetooth transport: BLE GATT `DataLink`s over `ble-gatt`'s datagram tier
//! (github.com/VRuzhentsov/ble-gatt).
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
//! not whether Bluetooth connects at all. Unlike `tcp_ws` (backed by the
//! mDNS/UDP presence worker) and `sim` (statically configured ports), there
//! is no discovery step here — candidates come from stored per-peer
//! Bluetooth metadata (`paired_devices.bluetooth_address`, gated on
//! `bluetooth_enabled` and a live OS-pairing check), exactly what
//! `pairing::commands::bluetooth_address_is_os_paired` already
//! checks for the enable command.

use std::collections::{HashMap, HashSet};
#[cfg(any(feature = "ui-plane", test))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

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
use crate::services::communication::channel::{BoxDialFuture, DataLink, Transport, ChannelKind};

mod search;

/// Fini's own GATT service/characteristic for the datagram tier. Fixed, not
/// user-configurable: both sync peers must advertise/expect the same UUIDs
/// to find each other's service. Distinct from any third-party device's own
/// UUIDs — this is Fini-to-Fini only, the app-to-app case `ble-gatt`'s
/// ADR-0003 describes.
const FINI_BLE_SERVICE_UUID: &str = "b1e6a000-f101-4000-8000-00805f9b34fb";
const FINI_BLE_CHARACTERISTIC_UUID: &str = "b1e6a001-f101-4000-8000-00805f9b34fb";

fn datagram_config() -> DatagramConfig {
    let mut config = DatagramConfig::new(
        ServiceUuid(Uuid::parse_str(FINI_BLE_SERVICE_UUID).expect("valid UUID literal")),
        CharacteristicUuid(Uuid::parse_str(FINI_BLE_CHARACTERISTIC_UUID).expect("valid UUID literal")),
    );
    if let Some(fingerprint) = local_fingerprint().get() {
        let mut payload = Vec::with_capacity(1 + FINGERPRINT_LEN);
        payload.push(if *add_mode_sender().borrow() {
            ADD_MODE_FLAG_BYTE
        } else {
            0
        });
        payload.extend_from_slice(fingerprint);
        config.advertised_manufacturer_data.insert(FINI_MANUFACTURER_ID, payload);
    }
    config
}

/// This device's advertised identity fingerprint, set once by `run_server`
/// before it first advertises. A `OnceLock` rather than a parameter on
/// `datagram_config` because the advertisement is rebuilt from several
/// places (serve, and each add-mode toggle) that have no reason to know
/// about identity -- the same reason `add_mode_sender` is a global.
fn local_fingerprint() -> &'static OnceLock<[u8; FINGERPRINT_LEN]> {
    static FINGERPRINT: OnceLock<[u8; FINGERPRINT_LEN]> = OnceLock::new();
    &FINGERPRINT
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
const ADD_MODE_FLAG_BYTE: u8 = 0x01;

/// Per-candidate cap for a dial+probe+reply confirmation round trip
/// (`hello_candidate`/`probe_discovery_hello`), separate from the overall
/// scan deadline: without this, a single candidate that accepts the
/// connection but never replies could consume the *entire* remaining scan
/// budget, starving out every other candidate that might otherwise have
/// matched sooner -- including the actual peer being searched for.
/// Still shorter than `AddDeviceView.vue`'s own per-pass scan duration
/// (`BLUETOOTH_SCAN_DURATION_MS`, currently 60s), so a slow candidate cannot
/// quietly consume a whole pass -- but no longer *much* shorter, because the
/// round trip it caps now has a third stage. `BleDataLink::send` retries a
/// `GattBusy` rejection for up to ~1.4s (see its comment), on top of a dial
/// that measurably takes 1-2s against a real phone. At the previous 1.5s this
/// budget could not fit dial + retry, so the retry was cut off mid-flight
/// every time and existed only on paper; the caller dropping the future also
/// cancels ble-gatt's own connect timeout, which is why those attempts left
/// no outcome in the logs at all.
///
/// The trade this accepts: with several candidates, one silent peer can now
/// take most of a pass. That is the lesser evil -- a probe too short to ever
/// complete fails *every* candidate, not just the ones behind a slow one.
///
/// Raised from 3s to 12s after instrumenting it on hardware: dial + hello +
/// reply took 2.4-3.9s against a Pixel, so 3s cut off the reply every time,
/// and the 4s scan window left the probe phase only ~2s of budget besides.
#[cfg(any(feature = "ui-plane", test))]
const CANDIDATE_PROBE_TIMEOUT: Duration = Duration::from_millis(12_000);

/// Once the listening phase of a candidate scan has heard its first
/// advertiser, it keeps listening this much longer (for a second device,
/// say) and then moves on to probing. The scan window is long (a minute,
/// `BLUETOOTH_SCAN_DURATION_MS`) so a slow adapter gets time, but a
/// candidate is only reported when the whole call returns: without this the
/// picker would sit empty for the full listening half after the peer was
/// already heard.
#[cfg(any(feature = "ui-plane", test))]
const ADVERTISER_SETTLE: Duration = Duration::from_secs(3);

/// Pairing legs (request / accept / complete) in progress. The add-mode
/// candidate scan yields to them: it holds a discovery session and
/// re-dials the same phone every pass, and an adapter doing either while a
/// pairing dial runs refuses or stalls it -- ble-gatt answered the pair dial
/// with "a dial to this peer is already in flight" and one dial hung until
/// abandoned, so the Pair button reported "Couldn't reach".
fn pairing_legs() -> &'static tokio::sync::watch::Sender<usize> {
    static LEGS: OnceLock<tokio::sync::watch::Sender<usize>> = OnceLock::new();
    LEGS.get_or_init(|| tokio::sync::watch::channel(0).0)
}

/// Pairing legs held right now.
#[cfg(test)]
pub(crate) fn pairing_legs_held() -> usize {
    *pairing_legs().borrow()
}

/// Serializes tests that hold a `PairingLeg` or assert on how many are held:
/// the count is process-global.
#[cfg(test)]
pub(crate) static PAIRING_LEGS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Held while a pairing leg or a Bluetooth channel setup runs; see
/// `pairing_legs`.
pub struct PairingLeg(());

impl PairingLeg {
    pub fn begin() -> Self {
        pairing_legs().send_modify(|count| *count += 1);
        Self(())
    }
}

impl Drop for PairingLeg {
    fn drop(&mut self) {
        pairing_legs().send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// How long the device with the higher id waits, in a Bluetooth setup, for the
/// other device's hello to reach it before dialling anyway. See
/// `setup_hello_round`.
#[cfg(all(feature = "ui-plane", not(test)))]
const HIGHER_ID_PATIENCE: Duration = Duration::from_secs(15);
#[cfg(test)]
const HIGHER_ID_PATIENCE: Duration = Duration::from_millis(1_500);

/// How long a pairing dial keeps retrying a refusal caused by the candidate
/// scan's cancelled probe still tearing down its connection.
const PAIRING_DIAL_RETRY_WINDOW: Duration = Duration::from_secs(5);

/// Dials `address` for a pairing leg and hands back the link plus the
/// registrations that keep it clear of the candidate scan: a pass stops at
/// its next step, and no scan runs until the caller drops the returned guards.
pub async fn dial_for_pairing(
    address: &str,
) -> Result<(Box<dyn DataLink>, impl Sized), String> {
    let leg = PairingLeg::begin();
    // A candidate probe already dialling is let finish, not cancelled; wait
    // for it so this dial never overlaps one to the same peer.
    drop(candidate_probe_lock().lock().await);
    let dial_guard = search::DialGuard::acquire().await;
    let started = tokio::time::Instant::now();
    loop {
        match dial(address).await {
            Ok(link) => return Ok((link, (leg, dial_guard))),
            // The probe's connection may still be closing.
            Err(_) if started.elapsed() < PAIRING_DIAL_RETRY_WINDOW => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Held for the length of one candidate probe; see `dial_for_pairing`.
fn candidate_probe_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// `setup_hello_round`'s per-candidate cap for one dial + hello + ack.
///
/// Against a real phone the dial alone takes 1-2s, and `BleDataLink::send`
/// retries a `GattBusy` rejection for up to ~1.4s on top; 4s was once too
/// short and dropped every attempt mid-connect:
///
/// ```text
/// 02:05:13  connect: dialling 52:E1:52:02:7D:37
/// 02:05:17  connect: abandoned before completing (connect guard dropped)
/// ```
///
/// 15s leaves a 60s setup round room for four candidates -- more than a
/// room ever holds.
#[cfg(any(feature = "ui-plane", test))]
const FIND_PEER_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(15);

/// Shared add-mode state, watched by `run_server`'s peripheral loop so a
/// toggle can trigger a fresh advertisement carrying (or dropping) the
/// add-mode flag without restarting the whole peripheral task -- Android's
/// `start_peripheral_once` is deliberately a one-time start (ADR-0002 in
/// ble-gatt: constructing `AndroidBackend` outside a genuine post-startup
/// call panics), so the *outer* task can never be torn down and re-spawned
/// to pick up a config change; only the inner advertise/accept loop can be.
///
/// `watch::Sender` alone is enough: it has its own `borrow()` for a
/// snapshot read (`datagram_config()`, above), and `subscribe()` hands out
/// a fresh `Receiver` for whichever caller needs to *wait* on a change
/// (`run_server`, in a `tokio::select!` against the incoming-connections
/// stream) — no need to also keep a shared `Receiver` around.
fn add_mode_sender() -> &'static tokio::sync::watch::Sender<bool> {
    static SENDER: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();
    SENDER.get_or_init(|| tokio::sync::watch::channel(false).0)
}

/// Called from `device_connection_enter_add_mode`/`leave_add_mode` — see
/// `add_mode_sender`'s doc comment for why this signals a running
/// `run_server` rather than restarting it.
pub fn set_add_mode(enabled: bool) {
    add_mode_sender().send_if_modified(|current| {
        if *current == enabled {
            return false;
        }
        *current = enabled;
        true
    });
}

/// Serializes test access to the `add_mode_sender` process-global: held by
/// this module's own test and by any `device_connection`/`transport` test
/// that goes through `enter_add_mode_impl`/`leave_add_mode_impl` (which also
/// call `set_add_mode`), so a concurrent flip from one can't land mid-assertion
/// in another.
#[cfg(test)]
pub(crate) static ADD_MODE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One `LinuxBackend` for the process's lifetime. `ble_gatt::backend::linux::LinuxBackend::new()`
/// opens a BlueZ D-Bus session and requires a powered adapter; constructing
/// it lazily (on first dial/serve attempt) rather than at startup means a
/// machine with no/unpowered Bluetooth adapter never fails app startup over
/// a transport most sessions won't use.
#[cfg(target_os = "linux")]
async fn backend() -> Result<Arc<dyn Backend>, String> {
    // `get_or_try_init`, not `get_or_init` over a cached `Result`: the
    // latter permanently bakes in whatever the *first* call returned,
    // success or failure. If Fini starts while Bluetooth is off, BlueZ is
    // restarting, or the adapter is briefly unavailable, that first
    // failure would be replayed to every later dial/serve attempt for the
    // rest of the process's life, even after the adapter comes back —
    // `get_or_try_init` instead leaves the cell empty on error, so the
    // next call genuinely retries construction.
    // Bounded: `LinuxBackend::new()` opens a D-Bus system-bus session and
    // talks to BlueZ over it. On a machine with no `bluetoothd` running at
    // all (headless CI containers, minimal installs) there's nothing wrong
    // locally, but D-Bus's own service-activation/name-owner-wait semantics
    // can leave that call pending far longer than this transport is worth
    // blocking anything on — this runs at app startup via
    // `tauri::async_runtime::spawn`, so an unbounded wait here would tie up
    // an async worker thread for however long that takes. A timeout turns
    // "no adapter reachable" into a fast, retryable failure instead.
    const BACKEND_INIT_TIMEOUT: Duration = Duration::from_secs(5);

    static BACKEND: OnceCell<Arc<dyn Backend>> = OnceCell::const_new();
    BACKEND
        .get_or_try_init(|| async {
            #[cfg(feature = "devtools")]
            if let Ok(endpoint) = std::env::var("FINI_BLE_MOCK_BROKER") {
                return mock_backend(&endpoint).await;
            }
            match tokio::time::timeout(BACKEND_INIT_TIMEOUT, LinuxBackend::new()).await {
                Ok(Ok(backend)) => Ok(Arc::new(backend) as Arc<dyn Backend>),
                Ok(Err(err)) => Err(err.to_string()),
                Err(_) => Err(format!(
                    "timed out after {BACKEND_INIT_TIMEOUT:?} waiting for the Bluetooth adapter"
                )),
            }
        })
        .await
        .map(Arc::clone)
}

/// The `actors-ble` e2e lane's radio: a `MockBackend` talking to a
/// cross-process broker instead of BlueZ, so two real `fini-app` processes
/// can prove this file's dial/peripheral/session-claim code path without
/// hardware. See `specs/e2e/actors/helpers/ble-sync.ts` and ble-gatt's
/// `docs/adr/0004-mock-broker-for-cross-process-e2e.md`.
///
/// Both the `devtools` feature gate above (off in release builds) and this
/// env var must be true for a process to ever reach this branch -- an env
/// var alone can never redirect a shipped binary onto a socket.
#[cfg(all(target_os = "linux", feature = "devtools"))]
async fn mock_backend(endpoint: &str) -> Result<Arc<dyn Backend>, String> {
    use ble_gatt::backend::mock::{MockBackend, MockNetwork};
    use ble_gatt::CapabilityReport;

    // Reuses the existing "fake local adapter address" escape hatch
    // (`pairing::commands::local_bluetooth_address`'s own env
    // override) rather than adding a second one -- the harness already sets
    // this per actor for the self-report path.
    let address = std::env::var("FINI_LOCAL_BLUETOOTH_ADDRESS")
        .map_err(|_| "FINI_BLE_MOCK_BROKER is set but FINI_LOCAL_BLUETOOTH_ADDRESS is not".to_string())?;
    let network = MockNetwork::remote(endpoint)
        .await
        .map_err(|err| format!("ble mock broker connect to {endpoint} failed: {err}"))?;
    Ok(Arc::new(MockBackend::new(
        PeerAddress(address),
        network,
        CapabilityReport { central: true, peripheral: true },
    )) as Arc<dyn Backend>)
}

/// The backend `tauri-plugin-ble-gatt` builds on Android: one for the
/// process, constructed on first use rather than at `.setup()`, where the
/// Activity context it needs does not exist yet (see the plugin's `lazy`
/// module). First use comes from a post-startup command or tick
/// (`start_peripheral_once`, `start_exchange`).
#[cfg(target_os = "android")]
async fn backend() -> Result<Arc<dyn Backend>, String> {
    crate::services::ble_plugin::backend()
}

/// Starts the Bluetooth peripheral acceptor loop exactly once. Android-only:
/// on Linux `lib.rs` spawns `run_server` unconditionally from `.setup()`,
/// which is safe there since `LinuxBackend::new()` has no Android-context
/// ordering requirement. On Android that same eager spawn would race
/// `tao`'s own context bring-up (see `backend`), so the
/// first call instead comes from `space_sync_tick_impl` — a
/// `#[tauri::command]`, whose first real invocation can only happen once
/// the WebView/Activity has actually dispatched an IPC call, a strictly
/// later and safer point than anything obtainable from `.setup()` itself.
/// The `Once` stays as a cheap filter -- this runs from every tick, and
/// without it each one would spawn a task only for `claim_peripheral_role`
/// to turn it away. The guard below is the actual invariant; this just
/// keeps the common path quiet.
#[cfg(target_os = "android")]
pub fn start_peripheral_once(state: DeviceConnectionState, db_path: PathBuf) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        tauri::async_runtime::spawn(run_server(state, db_path));
    });
}

/// One peripheral acceptor per process, enforced where the loop actually
/// runs rather than at each call site.
///
/// This used to be a `Once` inside `start_peripheral_once`, which guarded
/// only that one caller. `GattRadio::serve` spawned `run_server` directly
/// as well, so on Android both ran: two accept loops, both handed the same
/// inbound central, both running the gate on it. One claimed the session,
/// the other was rejected as a duplicate, and dropping the rejected link
/// released the session the winner was using.
///
/// Guarding the loop itself makes a second one impossible whatever calls
/// it, on any platform -- which is also what makes the invariant testable
/// without an Android device (see `peripheral_role_tests`).
#[cfg(any(feature = "ui-plane", test))]
fn claim_peripheral_role() -> bool {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    !RUNNING.swap(true, Ordering::SeqCst)
}

pub struct BleDataLink {
    channel: DatagramChannel,
    peer_addr: String,
}

impl BleDataLink {
    fn new(channel: DatagramChannel) -> Self {
        let peer_addr = channel.peer().0.clone();
        Self { channel, peer_addr }
    }
}

#[async_trait]
impl DataLink for BleDataLink {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn peer_addr(&self) -> Option<String> {
        Some(self.peer_addr.clone())
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        // Retry `GattBusy` here rather than inside `ble-gatt`: the backend
        // makes exactly one attempt and classifies a rejection it believes is
        // transient as `BleError::GattBusy`, deliberately leaving the retry
        // budget to whoever knows the caller's own deadline. No single
        // backend-side budget can be right for every caller -- a chain sized
        // for a generous one silently exceeds a tight one and reads as a
        // caller-abandoned future rather than an honest failure.
        //
        // Sized to the tightest caller on this path: `scan_add_mode_candidates`
        // gets `BLUETOOTH_SCAN_DURATION_MS` (4s) for the *whole* pass, dial
        // included, and `setup_hello_round` allows `FIND_PEER_CANDIDATE_TIMEOUT`
        // (4s) per candidate. With a dial typically eating 1-2s of that, the
        // ~1.4s worst case below still leaves the caller room to fail cleanly
        // instead of being cut off mid-retry.
        //
        // Observed on real hardware: the *first* write on a freshly connected
        // channel is the one that gets rejected (msg_id=0, fragment 0), while
        // the `subscribe` moments earlier on the same link succeeds -- so this
        // covers a genuine just-connected window, not a dead peer.
        const SEND_RETRY_DELAYS: [Duration; 3] = [
            Duration::from_millis(150),
            Duration::from_millis(300),
            Duration::from_millis(600),
        ];

        let mut attempt = 0;
        loop {
            match self.channel.send(payload.clone()).await {
                Ok(()) => return Ok(()),
                Err(ble_gatt::BleError::GattBusy(err)) if attempt < SEND_RETRY_DELAYS.len() => {
                    log::warn!(
                        "[transport][ble] send to {} rejected as busy ({err}), retrying ({}/{})",
                        self.peer_addr,
                        attempt + 1,
                        SEND_RETRY_DELAYS.len()
                    );
                    tokio::time::sleep(SEND_RETRY_DELAYS[attempt]).await;
                    attempt += 1;
                }
                Err(err) => return Err(err.to_string()),
            }
        }
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        match self.channel.recv().await? {
            Ok(bytes) => Some(Ok(bytes)),
            Err(err) => Some(Err(err.to_string())),
        }
    }
}

/// Central role: dial a peer's Bluetooth address.
pub async fn dial(address: &str) -> Result<Box<dyn DataLink>, String> {
    let backend = backend().await?;
    let peer = PeerAddress(address.to_string());
    let channel = datagram::connect(backend, &peer, &datagram_config())
        .await
        .map_err(|err| format!("ble connect to {address} failed: {err}"))?;
    Ok(Box::new(BleDataLink::new(channel)))
}

/// Scans for nearby Fini BLE advertisers and opportunistically connects and
/// authenticates each discovered address against `peer_id`'s already-known
/// `device_id` — the "discover" half of Phase 1 in
/// `docs/adr/0002-bluetooth-address-exchange-live-status-and-ble-pairing.md`,
/// used when this side cannot self-report its own address (Android) or a
/// peer just hasn't sent one yet. A real `AuthOk` from a candidate is what
/// proves it belongs to the expected peer, not merely some other nearby
/// Fini install; `backend.scan()` itself already filters to Fini's service
/// UUID (each backend does this at the native scan-callback level, before
/// candidates ever reach this Rust code).
///
/// On success the address is persisted — and Bluetooth enabled for the
/// pair, if this machine's own OS bonding with it already exists — via
/// `persist_bluetooth_address_and_maybe_enable`. Returns the confirmed
/// address, or `None` if nothing matched within `timeout`. The confirming
/// connection is dropped either way: this function's job is identity
/// confirmation, not establishing the real session — the next
/// `space_sync_tick`'s dial loop picks the now-eligible peer up normally.
/// Dials `address` and sends this device's hello (ADR-0008 D1). `Some(())`
/// if the device there is `peer_id` and acknowledged it -- which it does
/// only while it is running its own setup search for us (D2).
#[cfg(any(feature = "ui-plane", test))]
async fn hello_candidate(state: &DeviceConnectionState, address: &str, peer_id: &str) -> Option<()> {
    let mut link = dial(address).await.ok()?;
    send_frame(
        link.as_mut(),
        &PeerFrame::Hello {
            device_id: state.identity.device_id.clone(),
        },
    )
    .await
    .ok()?;
    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::HelloAck { device_id })) if device_id == peer_id => Some(()),
        _ => None,
    }
}

/// One Bluetooth setup-search round for `peer_id` (ADR-0008 D12): asks the
/// search coordinator for the peer for up to `timeout`, and says hello the
/// moment it is heard. `Ok(true)` if the peer acknowledged; `Err` when the
/// radio itself is unusable, so the caller can pause.
#[cfg(any(feature = "ui-plane", test))]
pub async fn setup_hello_round(
    state: DeviceConnectionState, peer_id: String, timeout: Duration,
) -> Result<bool, String> {
    let Some(found) = search::find(&peer_id, search::Purpose::Setup, timeout).await else {
        if is_bluetooth_adapter_unavailable() {
            return Err("bluetooth adapter unavailable".to_string());
        }
        return Ok(false);
    };
    // Bounded: a candidate that accepts the connection and never answers
    // must not hold the round open.
    // Both devices search for each other, and each dials the other the moment
    // it hears it. Two dials crossing fail each other: a device that is
    // itself mid-connect is not connectable, so the other's dial hangs until
    // BlueZ aborts it ("le-connection-abort-by-local" after 8-12 s), and the
    // retry crosses again. Measured on a laptop and a Pixel, this was most of
    // the 40-120 s a setup took. So the device with the higher id lets the
    // other dial first, and dials back once the other's hello has reached it.
    wait_for_turn_to_dial(&state, &peer_id).await;
    // A candidate probe already dialling is let finish, not cancelled; this
    // dial must not overlap one to the same peer.
    let _probe = candidate_probe_lock().lock().await;
    let acknowledged = tokio::time::timeout(
        FIND_PEER_CANDIDATE_TIMEOUT,
        hello_candidate(&state, &found.address, &peer_id),
    )
    .await
    .ok()
    .flatten()
    .is_some();
    if !acknowledged {
        search::note_dial_failed(&found.address);
    }
    Ok(acknowledged)
}

/// In a Bluetooth setup the device with the lower id dials first; the one
/// with the higher id waits until the other's hello has reached it (or
/// `HIGHER_ID_PATIENCE` runs out) before dialling back. See
/// `setup_hello_round`.
#[cfg(any(feature = "ui-plane", test))]
pub(crate) async fn wait_for_turn_to_dial(state: &DeviceConnectionState, peer_id: &str) {
    if state.identity.device_id.as_str() <= peer_id {
        return;
    }
    let patience = tokio::time::Instant::now() + HIGHER_ID_PATIENCE;
    while tokio::time::Instant::now() < patience
        && !state
            .channel_setup(peer_id, ChannelKind::Bluetooth)
            .is_some_and(|setup| setup.acked_peer_hello)
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A nearby, not-yet-paired device discovered via BLE while both sides are
/// in add-mode — the Bluetooth-side entry `AddDeviceView.vue`'s unified
/// candidate list merges alongside mDNS-discovered ones (ADR 0002 Phase 3).
#[cfg(any(feature = "ui-plane", test))]
pub struct AddModeCandidate {
    pub address: String,
    pub device_id: String,
    pub hostname: String,
}

/// Dials `address` and exchanges `DiscoveryHello`/`DiscoveryHelloReply`.
/// `None` on any failure along the way (dial, send, no/wrong reply); the
/// caller is responsible for bounding how long this is allowed to run.
#[cfg(any(feature = "ui-plane", test))]
async fn probe_discovery_hello(address: &str) -> Option<PeerFrame> {
    // The add-mode scan has closed by now, but a search for a paired peer
    // may start one; registering the dial keeps it paused until we finish.
    let _probe = candidate_probe_lock().lock().await;
    let _dial = search::DialGuard::acquire().await;
    let mut link = dial(address).await.ok()?;
    send_frame(link.as_mut(), &PeerFrame::DiscoveryHello).await.ok()?;
    recv_frame(link.as_mut()).await?.ok()
}

/// Scans for nearby Fini BLE advertisers carrying the add-mode flag (see
/// `datagram_config`/`set_add_mode`) and exchanges `DiscoveryHello` with
/// each one to learn its identity — Phase 3's discovery mechanism for
/// devices that have never paired at all. Devices not currently
/// advertising the flag are invisible here and never connected to; this is
/// the client-side half of the filtering `datagram_config` implements on
/// the advertising side.
///
/// Returns everything found within `timeout`, not just the first match
/// (unlike `setup_hello_round`, this feeds a picker list, not a single
/// confirm-and-persist action) — callers needing an ongoing view call this
/// repeatedly rather than once for a long window.
#[cfg(any(feature = "ui-plane", test))]
pub async fn scan_add_mode_candidates(
    my_device_id: &str, timeout: Duration,
) -> Result<Vec<AddModeCandidate>, String> {
    let mut legs = pairing_legs().subscribe();
    // Wait out a pairing leg already running, then run the pass and give it
    // up when one starts. An `Err` keeps the picker's previous list (the
    // caller treats it as "retry later"), where an empty `Ok` would wipe the
    // very candidate being paired.
    let _ = legs.wait_for(|count| *count == 0).await;
    let backend = backend().await.inspect_err(|_| note_adapter_unreachable())?;
    scan_add_mode_candidates_pass(my_device_id, timeout, &mut legs, backend, |address| async move {
        probe_discovery_hello(&address).await
    })
    .await
}

#[cfg(any(feature = "ui-plane", test))]
const PAIRING_PAUSED: &str = "candidate scan paused: a pairing step is running";

#[cfg(any(feature = "ui-plane", test))]
async fn scan_add_mode_candidates_pass<Probe, Reply>(
    my_device_id: &str,
    timeout: Duration,
    legs: &mut tokio::sync::watch::Receiver<usize>,
    backend: Arc<dyn Backend>,
    probe: Probe,
) -> Result<Vec<AddModeCandidate>, String>
where
    Probe: Fn(String) -> Reply,
    Reply: std::future::Future<Output = Option<PeerFrame>>,
{
    use futures_util::StreamExt;

    let deadline = tokio::time::Instant::now() + timeout;

    // Two phases, and the split is the point: listen to completion, close
    // the scan, *then* probe. Probing while the discovery stream is still
    // open asks one adapter to run active discovery and establish a
    // connection at the same moment, and it does not do both -- measured on
    // this hardware in the dial path, where BlueZ answered `Connect` with
    // nothing at all until ble-gatt's own 20s timeout fired, every single
    // attempt, against a peer at rssi -62.
    //
    // This function had the same shape and is the prime suspect for why
    // in-app BLE pairing has never worked. Unverified on hardware: the fix
    // is mechanical and mirrors `connect_by_advertisement`, but the symptom
    // it is meant to cure has only been observed in that sibling.
    let flagged_addresses = {
        let _scan = search::scan_lease_between_dials().await;
        let mut discovered = backend
            .scan(datagram_config().service)
            .await
            .inspect_err(|err| {
                note_scan_refused(err);
            })
            .map_err(|err| format!("ble scan failed: {err}"))?;
        let _running = RunningScan::start();

        // Listening gets at most half the caller's window, so the probe
        // phase always has something left. Splitting scan from probe fixed
        // one bug and introduced the risk of another: a scan that runs to
        // the full deadline leaves zero budget for the dials it just queued
        // up, and the pass returns nothing having done nothing -- looking
        // exactly like "no candidates" while actually meaning "no time".
        let mut listen_deadline = deadline - timeout / 2;

        let mut flagged: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            let remaining =
                listen_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            // Listening is safe to drop mid-way (it is only a discovery
            // session), so a pairing step takes the adapter at once.
            let next = tokio::select! {
                next = tokio::time::timeout(remaining, discovered.next()) => next,
                _ = legs.wait_for(|count| *count > 0) => {
                    return Err(PAIRING_PAUSED.to_string());
                }
            };
            let peer = match next {
                Ok(Some(Ok(peer))) => peer,
                // A backend-level scan failure (e.g. Android's async
                // `onScanFailed`) means Bluetooth itself is unusable, not
                // merely "no more candidates" -- propagate it like
                // `setup_hello_round` does, rather than reporting an
                // apparently-successful empty/partial scan.
                Ok(Some(Err(err))) => return Err(format!("ble scan failed: {err}")),
                // Timed out, or the stream ended: stop with whatever was
                // already found.
                Ok(None) | Err(_) => break,
            };
            let address = peer.address.0.clone();
            if !seen.insert(address.clone()) {
                continue;
            }
            if seen.len() == 1 {
                listen_deadline =
                    listen_deadline.min(tokio::time::Instant::now() + ADVERTISER_SETTLE);
            }
            // Deliberately probes *every* Fini advertiser, not only those
            // carrying the add-mode flag.
            //
            // The flag was only ever a cheap pre-filter. The authority is
            // the `DiscoveryHello` reply: `run_peer_gate` answers it solely
            // `if state.is_add_mode_enabled()`, so a device that is not in
            // add-mode stays silent and never becomes a candidate. Skipping
            // the pre-filter costs a dial per nearby Fini install and
            // changes no outcome.
            //
            // Why it is skipped: on hardware the desktop discovered the
            // phone repeatedly for two minutes and probed it zero times,
            // because the phone's advertisement did not carry the flag even
            // though the phone was in add-mode. Two attempts to explain that
            // were wrong, and pairing is blocked meanwhile. This trades an
            // unexplained filter for a slower but working discovery, and the
            // real fix belongs with the rest of BLE pairing in its own
            // change -- issue #169.
            //
            // Cost to be honest about: a room with several Fini devices
            // makes every Add Device scan dial all of them and wait out
            // `CANDIDATE_PROBE_TIMEOUT` on the ones that are not pairing.
            flagged.push(address);
        }
        // Logged unconditionally, at info. Three separate hypotheses about
        // why add-mode discovery finds nothing have now been wrong, each
        // costing a build/deploy/hardware cycle, because the only evidence
        // available was `scan: discovered` lines from ble-gatt that cannot
        // distinguish this scan from the dial loop's. This line says what
        // *this* call saw and what it will probe, which is the fact every
        // one of those attempts was missing.
        log::info!(
            "[transport][ble] add-mode scan saw {} advertiser(s), probing {}",
            seen.len(),
            flagged.len()
        );
        flagged
        // `discovered` is dropped here, stopping discovery, before any
        // probe below runs.
    };

    let mut candidates = Vec::new();
    for address in flagged_addresses {
        // Bounded by the *remaining* scan deadline, not a fixed window: one
        // unresponsive candidate (in range, advertising, but slow or gone
        // by the time this connects) must not eat the whole scan past the
        // caller's requested `duration_ms` -- the frontend runs this as a
        // single self-rescheduling chain, so one stuck candidate here would
        // otherwise delay every subsequent Add Device discovery pass. Dial
        // and send are covered too, not just the reply: neither has a bound
        // of its own. Also capped per-candidate (`CANDIDATE_PROBE_TIMEOUT`):
        // without that, one silent candidate could eat the *entire*
        // remaining budget by itself, starving out every other candidate
        // still to be tried, including the one actually being searched for.
        // A probe in flight is never cancelled for pairing: abandoning a
        // dial makes ble-gatt quarantine the address and disconnect it in
        // the background, which removes the device from BlueZ and fails the
        // pairing dial that follows. It finishes (bounded by
        // `CANDIDATE_PROBE_TIMEOUT`) and the pass stops before the next one.
        if *legs.borrow() > 0 {
            return Err(PAIRING_PAUSED.to_string());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let reply = tokio::time::timeout(
            remaining.min(CANDIDATE_PROBE_TIMEOUT),
            probe(address.clone()),
        )
        .await;
        if let Ok(Some(PeerFrame::DiscoveryHelloReply { device_id, hostname })) = reply {
            // A stale/self-seen advertisement (e.g. two adapters on the
            // same machine, or a previous scan's own peripheral still
            // winding down) must not show up as a candidate to pair with.
            if device_id != my_device_id {
                candidates.push(AddModeCandidate { address, device_id, hostname });
            }
        }
    }
    Ok(candidates)
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

    if !claim_peripheral_role() {
        log::warn!("[transport][ble] peripheral acceptor already running; ignoring a second start");
        return;
    }

    // Before the first `datagram_config()` below builds an advertisement:
    // the fingerprint is what lets a scanning peer tell this device apart
    // from any other Fini install without connecting to it first.
    let _ = local_fingerprint().set(fingerprint_of(&state.identity.device_id));

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

    let mut advertising_wanted = advertising_wanted_sender().subscribe();
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
        // `add_mode_sender`'s doc comment for why this is a signal to the
        // running loop rather than a full task restart.
        let mut add_mode_rx = add_mode_sender().subscribe();
        let mut incoming = match datagram::serve(backend, &datagram_config()).await {
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
                    let link: Box<dyn DataLink> = Box::new(BleDataLink::new(channel));
                    // Mirrors tcp_ws's/sim's "connection from {addr}" accept
                    // log -- without this, `run_peer_gate`'s own auth-outcome
                    // logging (added alongside this) has no matching "an
                    // attempt arrived at all" line to pair with, so a link
                    // that dies before ever sending an Auth frame (e.g. lost
                    // at the GATT layer before the first read) would leave no
                    // trace here that anything was accepted.
                    log::info!(
                        "[transport][ble] central connected: {}",
                        link.peer_addr().unwrap_or_default()
                    );
                    let state = state.clone();
                    let db_path = db_path.clone();
                    live_centrals.fetch_add(1, Ordering::SeqCst);
                    let live_centrals = live_centrals.clone();
                    let central_gone_tx = central_gone_tx.clone();
                    tokio::spawn(async move {
                        crate::services::communication::pairing::run_peer_gate(link, state, db_path).await;
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

/// How long a heard advertisement keeps a peer present: three status-search
/// windows, so one missed window does not flicker the row (ADR-0008 D12).
const BLUETOOTH_CHANNEL_TIMEOUT: Duration = Duration::from_secs(60);

/// When a delivery search does not find the peer, the next one waits this
/// long, one step further each time and then the last step forever: 1, 5
/// and 15 minutes, then every 15 (ADR-0008 D12). A first thoughtful guess
/// for battery, to be revised from measurements on hardware.
const DELIVERY_RETRY: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];

/// Whether this device is dialing the peer for an exchange right now.
#[cfg(any(feature = "ui-plane", test))]
pub fn dialing(peer_id: &str) -> bool {
    in_flight_exchanges().lock().unwrap().contains(peer_id)
}

/// Peers with an exchange attempt in flight -- one at a time per peer.
fn in_flight_exchanges() -> &'static StdMutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| StdMutex::new(HashSet::new()))
}

/// Set while a discovery session is actually running: `scan()` returned
/// Ok and the stream is alive. Holding `scan_lease` is not enough -- the
/// holder may still be waiting on an adapter that will refuse it.
static SCAN_RUNNING: AtomicBool = AtomicBool::new(false);

/// Marks a started discovery session; clears the mark when dropped.
struct RunningScan;

impl RunningScan {
    fn start() -> Self {
        SCAN_RUNNING.store(true, Ordering::SeqCst);
        note_adapter_reachable();
        Self
    }
}

impl Drop for RunningScan {
    fn drop(&mut self) {
        SCAN_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// One Bluetooth scan at a time, process-wide. Every `backend.scan` call
/// holds this for as long as its discovery stream lives.
///
/// Android's backend refuses a second scan while one is running ("a scan is
/// already active"), and each caller read that refusal as "the adapter is
/// unavailable" -- so the adapter check a person triggers by switching a
/// channel on, landing inside the dial loop's scan window, reported their
/// working Bluetooth as off, and the dial loop did the same in reverse.
/// Linux has no such refusal, but an adapter driving two discovery sessions
/// competes with itself (see `connect_by_advertisement`), so the lease
/// applies on every platform.
fn scan_lease() -> &'static tokio::sync::Mutex<()> {
    static LEASE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LEASE.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Per peer: how many delivery searches in a row found nothing, and when
/// the next one is due.
fn delivery_schedule() -> &'static StdMutex<HashMap<String, (usize, Instant)>> {
    static SCHEDULE: OnceLock<StdMutex<HashMap<String, (usize, Instant)>>> = OnceLock::new();
    SCHEDULE.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// When the next delivery search is due after `misses` searches in a row
/// found nothing.
fn next_delivery_search(misses: usize, now: Instant) -> Instant {
    now + DELIVERY_RETRY[misses.saturating_sub(1).min(DELIVERY_RETRY.len() - 1)]
}

/// Whether a delivery search for this peer is due. A present peer is always
/// due: connecting to it is one dial, not a search.
fn delivery_due(peer_id: &str) -> bool {
    if peer_seen_advertising_recently(peer_id) {
        return true;
    }
    match delivery_schedule().lock() {
        Ok(schedule) => schedule.get(peer_id).is_none_or(|(_, next)| Instant::now() >= *next),
        Err(_) => true,
    }
}

fn note_delivery_missed(peer_id: &str) {
    let now = Instant::now();
    let next = match delivery_schedule().lock() {
        Ok(mut schedule) => {
            let misses = schedule.get(peer_id).map_or(0, |(misses, _)| *misses) + 1;
            let next = next_delivery_search(misses, now);
            schedule.insert(peer_id.to_string(), (misses, next));
            next
        }
        Err(_) => return,
    };
    // The next search is due then; wake the keeper for it (ADR-0008 D12).
    crate::services::communication::sync::commands::notify_sync_work_pending_after(
        next.saturating_duration_since(now),
    );
}

/// See `ChannelService::forget_failures`: the next delivery search is due
/// at once. Also used when a pair is removed.
pub fn forget_delivery_misses(peer_id: &str) {
    note_delivery_reached(peer_id);
}

fn note_delivery_reached(peer_id: &str) {
    if let Ok(mut schedule) = delivery_schedule().lock() {
        schedule.remove(peer_id);
    }
}

/// Start an exchange with this peer over Bluetooth unless one is running or
/// being attempted (ADR-0008 D10). Called when there is work for the peer.
pub fn start_exchange(state: &DeviceConnectionState, peer_id: &str) {
    if state.has_session_on(peer_id, ChannelKind::Bluetooth) || !delivery_due(peer_id) {
        return;
    }
    // While a device is being added, the adapter belongs to that search.
    if *add_mode_sender().borrow() {
        return;
    }
    if !in_flight_exchanges().lock().unwrap().insert(peer_id.to_string()) {
        return;
    }
    let state = state.clone();
    let peer_id = peer_id.to_string();
    tauri::async_runtime::spawn(async move {
        exchange_with(&state, &peer_id).await;
        in_flight_exchanges().lock().unwrap().remove(&peer_id);
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

    if let Some(address) = last_seen_address(peer_id) {
        let guard = search::DialGuard::acquire().await;
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
            search::find(peer_id, search::Purpose::Delivery, remaining).await
        };
        let Some(found) = found else {
            match last_error {
                Some(err) => log::info!("[transport][ble] {peer_id} refused the exchange: {err}"),
                None => log::info!("[transport][ble] {peer_id} not heard within the search window"),
            }
            note_delivery_missed(peer_id);
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
        let mut link = dial(address).await?;
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
                search::note_dial_failed(address);
            }
            Err(err)
        }
        Err(_elapsed) => {
            log::info!("[transport][ble] candidate {address} did not finish connect+auth in time");
            search::note_dial_failed(address);
            Err("connect+auth timed out".to_string())
        }
    }
}

async fn run_exchange(
    state: &DeviceConnectionState, peer_id: &str, link: Box<dyn DataLink>, version: u32, address: &str,
) {
    note_delivery_reached(peer_id);
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

/// When each peer was last heard advertising a matching fingerprint, and at
/// which address. Only ever written from a fingerprint match, so "heard"
/// means "heard advertising *as this peer*".
fn last_seen_advertising() -> &'static StdMutex<HashMap<String, (Instant, String)>> {
    static LAST_SEEN: OnceLock<StdMutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
    LAST_SEEN.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn note_peer_advertising(peer_id: &str, address: &str) {
    let newly_present = match last_seen_advertising().lock() {
        Ok(mut seen) => seen
            .insert(peer_id.to_string(), (Instant::now(), address.to_string()))
            .is_none_or(|(previous, _)| previous.elapsed() >= BLUETOOTH_CHANNEL_TIMEOUT),
        Err(_) => false,
    };
    // A peer appearing is the moment waiting work becomes deliverable
    // (ADR-0008 D8). Only on the transition: a peer heard on every window
    // must not turn the keeper into a poll.
    if newly_present {
        crate::services::communication::sync::commands::notify_sync_work_pending();
    }
}

/// Where `peer_id` was last heard advertising, if within the channel
/// timeout: a present peer is reached by one dial, not a search.
fn last_seen_address(peer_id: &str) -> Option<String> {
    match last_seen_advertising().lock() {
        Ok(seen) => seen
            .get(peer_id)
            .filter(|(at, _)| at.elapsed() < BLUETOOTH_CHANNEL_TIMEOUT)
            .map(|(_, address)| address.clone()),
        Err(_) => None,
    }
}

/// Whether `peer_id` advertised within the channel timeout (ADR-0008 D9).
pub fn peer_seen_advertising_recently(peer_id: &str) -> bool {
    match last_seen_advertising().lock() {
        Ok(seen) => seen
            .get(peer_id)
            .is_some_and(|(at, _)| at.elapsed() < BLUETOOTH_CHANNEL_TIMEOUT),
        Err(_) => false,
    }
}

/// Turn the status search on while the Device page is open, off when it
/// closes. Green is never computed in the background (ADR-0008 D12); the
/// search coordinator merges it with any delivery or setup search running.
#[cfg(any(feature = "ui-plane", test))]
pub fn set_status_search(state: &DeviceConnectionState, active: bool) {
    search::set_status(active.then(|| state.db_path.clone()));
}

/// Whether this device should advertise (ADR-0008 D8): while it has a
/// Bluetooth channel on, is setting one up, or is being paired. Otherwise
/// the radio stays quiet -- there is nobody it should be reachable by.
fn advertising_wanted_sender() -> &'static tokio::sync::watch::Sender<bool> {
    static SENDER: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();
    SENDER.get_or_init(|| tokio::sync::watch::channel(false).0)
}

/// How long this device stays reachable after a Bluetooth init completes
/// here. Its ack to the peer's hello can be lost when the link drops, so the
/// peer may still be searching; the gate still answers it (ADR-0008 D2), but
/// only if the peer can find this device.
#[cfg(any(feature = "ui-plane", test))]
const ANSWER_AFTER_SETUP: Duration = Duration::from_secs(120);

fn answer_after_setup_until() -> &'static std::sync::Mutex<Option<std::time::Instant>> {
    static UNTIL: OnceLock<std::sync::Mutex<Option<std::time::Instant>>> = OnceLock::new();
    UNTIL.get_or_init(|| std::sync::Mutex::new(None))
}

fn answering_after_setup() -> bool {
    answer_after_setup_until()
        .lock()
        .ok()
        .and_then(|until| *until)
        .is_some_and(|until| std::time::Instant::now() < until)
}

/// A Bluetooth init completed here: keep advertising for a while, then look
/// again whether anything still wants it.
#[cfg(any(feature = "ui-plane", test))]
pub fn keep_answering_after_setup() {
    if let Ok(mut until) = answer_after_setup_until().lock() {
        *until = Some(std::time::Instant::now() + ANSWER_AFTER_SETUP);
    }
    crate::services::communication::sync::commands::notify_sync_work_pending_after(
        ANSWER_AFTER_SETUP + Duration::from_secs(1),
    );
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
    let wanted = any_channel_on
        || state.any_channel_setup(ChannelKind::Bluetooth)
        || answering_after_setup()
        || *add_mode_sender().borrow();
    advertising_wanted_sender().send_if_modified(|current| {
        let changed = *current != wanted;
        *current = wanted;
        changed
    });
}

/// Whether the *local* Bluetooth radio could actually be used the last time
/// this process tried to use it.
///
/// Three-valued on purpose. Before anything has been attempted the honest
/// answer is "nobody has looked", and a row must not tell the user their
/// adapter is off on that basis -- the same reasoning that makes
/// `bluetooth_peer_nearby_now` default to `true` on a platform that cannot
/// look.
///
/// Deliberately *observed* rather than polled. Asking BlueZ whether the
/// adapter is powered costs a D-Bus round trip, and the transport row is
/// polled every few seconds for every paired peer; the dial loop already
/// searches whenever it has a reason to, so recording what those attempts find
/// keeps this fresh for free. It is also the truer signal: "we asked the
/// radio to do something and it refused" is what the user actually cares
/// about, and it catches an adapter that reports itself powered while
/// refusing to scan.
///
/// Only a real *use* records health. Recording success from `backend()`
/// alone would be wrong: it caches its `Arc` in a `OnceCell`, so once the
/// adapter has worked once, every later call returns that cached handle
/// without touching hardware -- an adapter switched off afterwards would
/// still look reachable, and the row would flap between the cached success
/// and the scan failure that follows it.
const ADAPTER_UNKNOWN: u8 = 0;
const ADAPTER_REACHABLE: u8 = 1;
const ADAPTER_UNREACHABLE: u8 = 2;

static ADAPTER_HEALTH: AtomicU8 = AtomicU8::new(ADAPTER_UNKNOWN);

fn note_adapter_reachable() {
    ADAPTER_HEALTH.store(ADAPTER_REACHABLE, Ordering::Relaxed);
}

fn note_adapter_unreachable() {
    ADAPTER_HEALTH.store(ADAPTER_UNREACHABLE, Ordering::Relaxed);
}

/// Records what a refused scan says about the adapter. A busy adapter (a
/// discovery already running, which ble-gatt has tried to recover) is
/// there and working -- only something else refuses it as missing.
fn note_scan_refused(err: &ble_gatt::BleError) -> bool {
    if matches!(err, ble_gatt::BleError::AdapterBusy(_)) {
        note_adapter_reachable();
        true
    } else {
        note_adapter_unreachable();
        false
    }
}

/// `true` only once a genuine attempt has failed -- never merely because
/// nothing has been tried yet. See `ADAPTER_HEALTH`.
#[cfg(any(feature = "ui-plane", test))]
pub fn is_bluetooth_adapter_unavailable() -> bool {
    #[cfg(test)]
    if ADAPTER_PINNED_REACHABLE.with(|pinned| pinned.get()) {
        return false;
    }
    ADAPTER_HEALTH.load(Ordering::Relaxed) == ADAPTER_UNREACHABLE
}

#[cfg(test)]
thread_local! {
    static ADAPTER_PINNED_REACHABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// For a test on this thread that is not about the adapter: the process-wide
/// health is written by concurrent tests (and by any scan they start), so a
/// check that reads it would pass or fail with their timing.
#[cfg(test)]
pub fn pin_adapter_reachable_on_this_thread() {
    ADAPTER_PINNED_REACHABLE.with(|pinned| pinned.set(true));
}

/// Asks the radio, right now, whether it can be used -- and records the
/// answer into `ADAPTER_HEALTH` like any other attempt.
///
/// The observed signal above is free but unhurried: it only refreshes when
/// the dial loop next scans, which since ADR-0007 is up to 30s away. That
/// is fine for a row correcting itself in the background and wrong for the
/// one moment the user is watching -- switching a channel on and expecting
/// to be told immediately if their own Bluetooth is off. So this exists to
/// be called from that user action, and only from it.
///
/// It opens a discovery session and drops it immediately rather than asking
/// BlueZ whether the adapter reports itself powered. An adapter that claims
/// to be powered and then refuses to scan is a real failure mode, and the
/// question worth answering is "can we do the thing", not "does the
/// hardware feel well".
///
/// A scan already running elsewhere answers the question without a second
/// one: the adapter accepted that discovery session, so it is reachable.
/// Starting our own would only be refused (see `scan_lease`). A caller that
/// merely holds the lease proves nothing yet -- its scan may still be
/// refused -- so the probe waits for it to start or finish.
#[cfg(any(feature = "ui-plane", test))]
pub async fn probe_adapter_available() -> bool {
    let deadline = tokio::time::Instant::now() + PROBE_WAIT_FOR_OTHER_SCAN;
    loop {
        if SCAN_RUNNING.load(Ordering::SeqCst) {
            note_adapter_reachable();
            return true;
        }
        if let Ok(_scan) = scan_lease().try_lock() {
            let Ok(backend) = backend().await else {
                note_adapter_unreachable();
                return false;
            };
            return match backend.scan(datagram_config().service).await {
                Ok(stream) => {
                    drop(stream);
                    note_adapter_reachable();
                    true
                }
                Err(err) => {
                    log::warn!("[transport][ble] adapter probe failed: {err}");
                    note_scan_refused(&err)
                }
            };
        }
        // Another caller holds the lease but its scan has not started: wait
        // for it to start (the adapter works) or give the lease up.
        if tokio::time::Instant::now() >= deadline {
            return !is_bluetooth_adapter_unavailable();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// How long the probe waits on another caller's scan that is still being
/// set up, before falling back to the last recorded adapter health.
#[cfg(any(feature = "ui-plane", test))]
const PROBE_WAIT_FOR_OTHER_SCAN: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    /// The peripheral acceptor must be a singleton, whatever starts it.
    ///
    /// It was not: `GattRadio::serve` spawned `run_server` on Linux *and*
    /// Android, while Android also started it from `dial` via
    /// `start_peripheral_once`. Two accept loops then received the same
    /// inbound central and both ran the gate on it -- one claimed the
    /// session, the other was rejected as a duplicate, and dropping the
    /// rejected link released the session the winner was using. Observed on
    /// hardware as an inbound link dying ~170ms after authenticating, with
    /// `no live notify session` as the only trace.
    ///
    /// The `cfg` fix alone is not testable off Android, since the wrong
    /// branch never compiles here. Guarding the loop itself is: the
    /// invariant stops being platform-conditional, and this asserts it.
    #[test]
    fn only_one_peripheral_acceptor_can_run_at_a_time() {
        assert!(claim_peripheral_role(), "the first start takes the role");
        assert!(
            !claim_peripheral_role(),
            "a second start must be refused -- two acceptors race each other's session"
        );
    }

    /// ADR-0008 D12: a peer not found is searched for again after 1, 5 and
    /// 15 minutes, then every 15 minutes.
    #[test]
    fn delivery_searches_back_off_one_five_fifteen_then_every_fifteen() {
        let now = Instant::now();
        let waits: Vec<u64> = (1..=5)
            .map(|misses| next_delivery_search(misses, now).duration_since(now).as_secs())
            .collect();
        assert_eq!(waits, vec![60, 300, 900, 900, 900]);
    }

    /// Switching Bluetooth on tries again at once, whatever retry delay
    /// earlier misses left behind.
    #[test]
    fn forgetting_delivery_misses_makes_the_next_search_due_now() {
        let peer = "peer-forget-delivery-misses";
        note_delivery_missed(peer);
        assert!(!delivery_due(peer), "a miss delays the next search");
        forget_delivery_misses(peer);
        assert!(delivery_due(peer));
    }

    /// `add_mode_sender` is a process-global singleton (mirrors the real
    /// adapter's own single peripheral instance). `device_connection`'s
    /// `enter_add_mode_impl`/`leave_add_mode_impl` also flip it, so any test
    /// exercising those (see `channel::tests`) must hold
    /// `ADD_MODE_TEST_LOCK` too, the same way other process-global test
    /// state in this crate is serialized.
    #[test]
    fn datagram_config_advertises_the_add_mode_flag_only_while_enabled() {
        let _guard = ADD_MODE_TEST_LOCK.lock().unwrap();
        // ADR-0006 slice 2 changed the shape this asserts. The payload is no
        // longer present only in add-mode and no longer equals a single
        // flag byte: it is a flags byte followed by the identity
        // fingerprint, advertised whenever a fingerprint is known, because
        // being identifiable is what lets a scanner skip peers it does not
        // want. The add-mode signal became bit 0 of that first byte.
        let _ = local_fingerprint().set(fingerprint_of("test-device"));
        let expected = *local_fingerprint().get().expect("fingerprint set above");

        set_add_mode(false);
        let disabled = datagram_config();
        let payload = disabled
            .advertised_manufacturer_data
            .get(&FINI_MANUFACTURER_ID)
            .expect("the fingerprint is advertised regardless of add-mode");
        assert_eq!(payload[0] & ADD_MODE_FLAG_BYTE, 0, "add-mode bit must be clear");
        assert_eq!(&payload[1..], &expected, "fingerprint must be advertised");

        set_add_mode(true);
        let enabled = datagram_config();
        let payload = enabled
            .advertised_manufacturer_data
            .get(&FINI_MANUFACTURER_ID)
            .expect("payload present in add-mode too");
        assert_eq!(
            payload[0] & ADD_MODE_FLAG_BYTE,
            ADD_MODE_FLAG_BYTE,
            "add-mode bit must be set while add-mode is on"
        );
        assert_eq!(&payload[1..], &expected, "fingerprint is unchanged by add-mode");

        set_add_mode(false);
        let disabled_again = datagram_config();
        assert_eq!(
            disabled_again.advertised_manufacturer_data.get(&FINI_MANUFACTURER_ID).map(|p| p[0]
                & ADD_MODE_FLAG_BYTE),
            Some(0),
            "must stop signalling add-mode once it is left again"
        );
    }

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

    /// Serializes tests that set `FINI_LOCAL_BLUETOOTH_ADDRESS`/
    /// `FINI_BLE_MOCK_BROKER` -- process-global env vars, unsafe to mutate
    /// from parallel test threads. Mirrors `BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK`
    /// in `pairing::commands`.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    static MOCK_BACKEND_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Proves `mock_backend` actually round-trips through a real socket to a
    /// real `MockNetwork::serve` broker, not just that it compiles -- the
    /// same connect+advertise+scan shape `ble-gatt`'s own
    /// `tests/mock_broker.rs` exercises, but through fini's injection point
    /// (`FINI_BLE_MOCK_BROKER`/`FINI_LOCAL_BLUETOOTH_ADDRESS`) instead of
    /// calling `MockNetwork::remote` directly. This is what the `actors-ble`
    /// e2e lane's two real `fini-app` processes will each do at startup.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    #[tokio::test]
    async fn mock_backend_connects_to_a_broker_and_reports_full_capabilities() {
        let _guard = MOCK_BACKEND_ENV_LOCK.lock().unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback broker port");
        let broker_addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = ble_gatt::backend::mock::MockNetwork::serve(listener).await;
        });

        std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "AA:BB:CC:00:00:01");
        let result = mock_backend(&broker_addr.to_string()).await;
        std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");

        let backend = result.expect("mock_backend should connect to the broker");
        let capabilities = backend.capabilities().await;
        assert!(capabilities.central && capabilities.peripheral);
    }

    /// The one precondition `mock_backend` itself enforces: without a local
    /// address there is nothing to identify this peer as on the mock radio,
    /// and the error must name which env var is missing rather than panic
    /// or silently pick something.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    #[tokio::test]
    async fn mock_backend_requires_a_local_bluetooth_address() {
        let _guard = MOCK_BACKEND_ENV_LOCK.lock().unwrap();
        std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");

        let err = match mock_backend("127.0.0.1:1").await {
            Ok(_) => panic!("must fail without a local address"),
            Err(err) => err,
        };
        assert!(err.contains("FINI_LOCAL_BLUETOOTH_ADDRESS"));
    }

    /// A mock radio with Fini advertisers at `addresses`, and a backend that
    /// scans it from its own address.
    fn mock_radio_with_advertisers(addresses: &[&str]) -> (Arc<dyn Backend>, Vec<Arc<dyn Backend>>) {
        use ble_gatt::backend::mock::{MockBackend, MockNetwork};
        use ble_gatt::CapabilityReport;

        let network = MockNetwork::new();
        let capabilities = CapabilityReport { central: true, peripheral: true };
        let scanner: Arc<dyn Backend> = Arc::new(MockBackend::new(
            PeerAddress("AA:00:00:00:01:00".to_string()),
            network.clone(),
            capabilities,
        ));
        let advertisers = addresses
            .iter()
            .map(|address| {
                Arc::new(MockBackend::new(PeerAddress(address.to_string()), network.clone(), capabilities))
                    as Arc<dyn Backend>
            })
            .collect();
        (scanner, advertisers)
    }

    async fn advertise_all(advertisers: &[Arc<dyn Backend>]) {
        for advertiser in advertisers {
            advertiser.advertise(datagram_config().service_spec()).await.expect("advertise on the mock radio");
        }
    }

    fn reply_from(address: &str) -> PeerFrame {
        PeerFrame::DiscoveryHelloReply {
            device_id: format!("device-{address}"),
            hostname: format!("host-{address}"),
        }
    }

    /// A pass that finds a pairing step already running gives up with the
    /// paused error -- which keeps the picker's list -- and dials nobody.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_candidate_scan_stands_aside_while_a_pairing_step_runs() {
        let _legs = PAIRING_LEGS_TEST_LOCK.lock().await;
        let (scanner, advertisers) = mock_radio_with_advertisers(&["AA:00:00:00:01:01"]);
        advertise_all(&advertisers).await;
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let leg = PairingLeg::begin();
        let mut legs = pairing_legs().subscribe();
        let result = scan_add_mode_candidates_pass("me", Duration::from_secs(10), &mut legs, scanner, |address| {
            let probes = probes.clone();
            async move {
                probes.fetch_add(1, Ordering::SeqCst);
                Some(reply_from(&address))
            }
        })
        .await;
        drop(leg);

        assert_eq!(result.err().as_deref(), Some(PAIRING_PAUSED));
        assert_eq!(probes.load(Ordering::SeqCst), 0, "no candidate is dialled during a pairing step");
    }

    /// A new pass does not start while a pairing step runs: it waits for it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_candidate_scan_waits_for_a_running_pairing_step() {
        let _legs = PAIRING_LEGS_TEST_LOCK.lock().await;
        let leg = PairingLeg::begin();
        let scan = tokio::spawn(scan_add_mode_candidates("me", Duration::from_millis(50)));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!scan.is_finished(), "the pass waits while the pairing step runs");
        drop(leg);
        // What the pass does once it may run depends on this machine's
        // adapter; only the wait is under test.
        scan.abort();
    }

    /// A probe already dialling when a pairing step starts is let finish:
    /// abandoning it makes ble-gatt quarantine the address and fails the
    /// pairing dial that follows. The pass then stops before the next probe.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_probe_in_flight_finishes_when_a_pairing_step_starts() {
        let _legs = PAIRING_LEGS_TEST_LOCK.lock().await;
        let (scanner, advertisers) = mock_radio_with_advertisers(&["AA:00:00:00:02:01", "AA:00:00:00:02:02"]);
        advertise_all(&advertisers).await;
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let finished = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());

        let pass = {
            let (started, finished, release) = (started.clone(), finished.clone(), release.clone());
            tokio::spawn(async move {
                let mut legs = pairing_legs().subscribe();
                scan_add_mode_candidates_pass("me", Duration::from_secs(10), &mut legs, scanner, move |address| {
                    let (started, finished, release) = (started.clone(), finished.clone(), release.clone());
                    async move {
                        started.fetch_add(1, Ordering::SeqCst);
                        release.notified().await;
                        finished.fetch_add(1, Ordering::SeqCst);
                        Some(reply_from(&address))
                    }
                })
                .await
            })
        };
        while started.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let leg = PairingLeg::begin();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!pass.is_finished(), "the probe in flight is not cancelled");
        release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(2), pass).await.unwrap().unwrap();
        drop(leg);

        assert_eq!(finished.load(Ordering::SeqCst), 1, "the probe in flight ran to its end");
        assert_eq!(started.load(Ordering::SeqCst), 1, "no probe starts after the pairing step began");
        assert_eq!(result.err().as_deref(), Some(PAIRING_PAUSED));
    }

    /// After a Bluetooth init completes here the device stays reachable a
    /// while, so a peer that missed its ack can still find it and ask again.
    #[test]
    fn a_finished_bluetooth_setup_keeps_advertising_a_while() {
        keep_answering_after_setup();
        assert!(answering_after_setup());
    }

    /// ble-gatt reports a discovery it could not take over as busy, not
    /// missing: that adapter works, so Bluetooth must not read as unavailable.
    #[test]
    fn a_busy_adapter_is_not_an_unavailable_one() {
        assert!(note_scan_refused(&ble_gatt::BleError::AdapterBusy("in progress".into())));
        assert!(!note_scan_refused(&ble_gatt::BleError::AdapterUnavailable("no adapter".into())));
    }

    /// A scan already running is proof the adapter works, so the check a
    /// person triggers by switching Bluetooth on must say so -- not start a
    /// second scan, which Android refuses ("a scan is already active") and
    /// which used to be recorded as "Bluetooth is unavailable on this device".
    #[tokio::test]
    async fn adapter_probe_during_another_scan_reports_the_adapter_reachable() {
        let _other_scan = scan_lease().lock().await;
        let _running = RunningScan::start();
        note_adapter_unreachable();

        assert!(
            probe_adapter_available().await,
            "a scan in flight means the adapter accepted a discovery session"
        );
        assert!(!is_bluetooth_adapter_unavailable());
    }

    /// Holding the lease is not a running scan. The add-mode scan takes the
    /// lease and then waits on the adapter; a probe in that window used to
    /// report a missing radio as working, so adding Bluetooth never failed.
    #[tokio::test(start_paused = true)]
    async fn adapter_probe_does_not_trust_a_scan_that_has_not_started() {
        let other_scan = scan_lease().lock().await;
        note_adapter_unreachable();

        let probe = tokio::spawn(probe_adapter_available());
        tokio::time::sleep(PROBE_WAIT_FOR_OTHER_SCAN + Duration::from_secs(1)).await;
        drop(other_scan);

        assert!(!probe.await.unwrap(), "no scan ever started, and the adapter was last seen unreachable");
    }
}
