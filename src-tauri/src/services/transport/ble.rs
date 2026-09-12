//! The Bluetooth transport: BLE GATT `Link`s over `ble-gatt`'s datagram tier
//! (github.com/VRuzhentsov/ble-gatt).
//!
//! Linux (BlueZ via `ble_gatt::backend::linux`) and Android (via
//! `ble_gatt::backend::android`, bridged through the same `tao` ->
//! `ndk-context` handoff `tauri-plugin-ble-gatt`'s own `android_lazy` module
//! uses for the JS-facing plugin — reimplemented here for this Rust-native
//! path, since Fini calls `ble-gatt` directly rather than through Tauri IPC).
//! See `android_lazy` below for why construction is deferred, and
//! `start_peripheral_once`/its caller in `space_sync::commands` for why the
//! peripheral role isn't spawned from `.setup()` on Android the way it is on
//! Linux.
//!
//! Plays the same role `transport::sim` plays for tests/E2E, but for real.
//! ADR-0003 revision: dials/accepts unconditionally, independent of
//! Network's own state -- both transports stay connected to a paired peer
//! at once, with `preferred_transport` only deciding which one is primary
//! (see `device_connection::DeviceConnectionState::recompute_primary_locked`),
//! not whether Bluetooth connects at all. Unlike `tcp_ws` (backed by the
//! mDNS/UDP presence worker) and `sim` (statically configured ports), there
//! is no discovery step here — candidates come from stored per-peer
//! Bluetooth metadata (`paired_devices.bluetooth_address`, gated on
//! `bluetooth_enabled` and a live OS-pairing check), exactly what
//! `device_connection::commands::bluetooth_address_is_os_paired` already
//! checks for the enable command.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use ble_gatt::backend::linux::LinuxBackend;
use ble_gatt::datagram::{self, DatagramChannel, DatagramConfig};
use ble_gatt::{Backend, CharacteristicUuid, PeerAddress, ServiceUuid};
use tokio::sync::OnceCell;
use uuid::Uuid;

/// Tauri's Android runtime (`tao`) keeps its own Android context separate
/// from the ecosystem-wide `ndk-context` interop point
/// `ble_gatt::backend::android::AndroidBackend::new()` reads. Bridging it,
/// and deferring the real backend's construction past `.setup()`, is
/// Tauri-specific glue that doesn't belong in `ble-gatt` itself — this is a
/// direct reimplementation of `tauri-plugin-ble-gatt`'s own `android_lazy`
/// module for Fini's Rust-native transport path (no Tauri IPC involved, so
/// that module's own JS-facing plugin code can't be reused directly).
///
/// See its doc comment for the crash this defers: `.setup()` runs
/// synchronously from inside `tao`'s own Android context bring-up, so
/// reading `ndk_context::android_context()` at that point panics with
/// "android context was not initialized". `LazyAndroidBackend` defers real
/// construction to the first genuine use, which for Fini means the first
/// `space_sync_tick` — see `start_peripheral_once` and its caller.
#[cfg(target_os = "android")]
mod android_lazy {
    use async_trait::async_trait;
    use ble_gatt::backend::android::AndroidBackend;
    use ble_gatt::{
        Backend, BleError, BoxStream, CapabilityReport, CharacteristicUuid, DiscoveredPeer,
        GattConnection, GattEvent, GattServiceSpec, PeerAddress, Result, ServiceUuid,
    };
    use tokio::sync::{broadcast, OnceCell};
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt;

    const EVENT_CHANNEL_CAPACITY: usize = 64;

    /// Delegates to the one shared bridge in `services::android_context` —
    /// `ndk_context::initialize_android_context` panics if invoked more than
    /// once for the process, and the OS-pairing check
    /// (`device_connection::commands::bluetooth_address_is_os_paired`) needs
    /// this same bridge from an independent call site, so both go through
    /// one idempotent entry point rather than each racing their own copy.
    fn bridge_ndk_context_from_tao() -> Result<()> {
        crate::services::android_context::ensure_bridged()
            .map_err(BleError::AdapterUnavailable)
    }

    pub struct LazyAndroidBackend {
        cell: OnceCell<AndroidBackend>,
        /// Events are republished through a channel owned by *this* wrapper,
        /// not borrowed from the inner backend. That is what lets a caller
        /// subscribe before the backend exists: `watchEvents()`/`events()`
        /// first is a natural setup order, and returning the inner
        /// backend's stream directly meant subscribing early got a
        /// permanently empty one — a silent, successful-looking no-op.
        events_tx: broadcast::Sender<GattEvent>,
    }

    impl LazyAndroidBackend {
        pub fn new() -> Self {
            let (events_tx, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
            Self { cell: OnceCell::new(), events_tx }
        }

        async fn inner(&self) -> Result<&AndroidBackend> {
            self.cell
                .get_or_try_init(|| async {
                    bridge_ndk_context_from_tao()?;
                    let backend = AndroidBackend::new().await?;
                    let mut source = backend.events();
                    let sink = self.events_tx.clone();
                    tokio::spawn(async move {
                        while let Some(event) = source.next().await {
                            // Errors when there are currently no receivers,
                            // which is normal, not terminal. Exiting on it
                            // meant one event arriving before anyone
                            // subscribed killed the forwarder permanently.
                            let _ = sink.send(event);
                        }
                    });
                    Ok(backend)
                })
                .await
        }
    }

    #[async_trait]
    impl Backend for LazyAndroidBackend {
        async fn capabilities(&self) -> CapabilityReport {
            match self.inner().await {
                Ok(backend) => backend.capabilities().await,
                Err(err) => {
                    log::warn!("[transport][ble] android backend construction failed: {err}");
                    CapabilityReport::default()
                }
            }
        }

        async fn scan(&self, service: ServiceUuid) -> Result<BoxStream<Result<DiscoveredPeer>>> {
            self.inner().await?.scan(service).await
        }

        async fn connect(&self, peer: &PeerAddress) -> Result<Box<dyn GattConnection>> {
            self.inner().await?.connect(peer).await
        }

        async fn advertise(&self, service: GattServiceSpec) -> Result<()> {
            self.inner().await?.advertise(service).await
        }

        async fn stop_advertising(&self) -> Result<()> {
            self.inner().await?.stop_advertising().await
        }

        async fn notify(&self, characteristic: CharacteristicUuid, value: Vec<u8>) -> Result<()> {
            self.inner().await?.notify(characteristic, value).await
        }

        async fn notify_peer(
            &self, peer: &PeerAddress, session: Option<u64>, characteristic: CharacteristicUuid,
            value: Vec<u8>,
        ) -> Result<()> {
            self.inner().await?.notify_peer(peer, session, characteristic, value).await
        }

        async fn disconnect_peer(&self, peer: &PeerAddress, session: Option<u64>) -> Result<()> {
            self.inner().await?.disconnect_peer(peer, session).await
        }

        fn events(&self) -> BoxStream<GattEvent> {
            // Always a live subscription, whether or not the backend exists
            // yet. Once it is built, `inner()` starts forwarding into this
            // channel.
            let rx = self.events_tx.subscribe();
            Box::pin(BroadcastStream::new(rx).map(|item| match item {
                Ok(event) => event,
                Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                    GattEvent::Lagged { dropped: n }
                }
            }))
        }
    }
}

use crate::services::db::open_db_at_path;
use crate::services::device_connection::{
    bluetooth_dial_candidates, note_observed_bluetooth_address as store_observed_bluetooth_address,
    DeviceConnectionState,
};
use crate::services::space_sync::session;
use crate::services::space_sync::types::PeerFrame;
use crate::services::transport::{recv_frame, send_frame, BoxDialFuture, Link, Transport, TransportKind};

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
/// (`probe_candidate`/`probe_discovery_hello`), separate from the overall
/// scan deadline: without this, a single candidate that accepts the
/// connection but never replies could consume the *entire* remaining scan
/// budget, starving out every other candidate that might otherwise have
/// matched sooner -- including the actual peer being searched for.
/// Still shorter than `AddDeviceView.vue`'s own per-pass scan duration
/// (`BLUETOOTH_SCAN_DURATION_MS`, currently 4s), so a slow candidate cannot
/// quietly consume a whole pass -- but no longer *much* shorter, because the
/// round trip it caps now has a third stage. `BleLink::send` retries a
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
const CANDIDATE_PROBE_TIMEOUT: Duration = Duration::from_millis(3_000);

/// `find_peer_address`'s own per-candidate cap, larger than
/// `CANDIDATE_PROBE_TIMEOUT`: `probe_candidate` tries a legacy
/// `perform_client_auth` fallback after `BluetoothProbe` goes unanswered
/// (see its doc comment), so a confirmation attempt here can be two
/// sequential dial+handshake round trips, not one. `find_peer_address`'s
/// own budget is the 60s "Find via Bluetooth" button timeout, not
/// `AddDeviceView.vue`'s tight 4s scan pass, so there's ample room for a
/// larger per-candidate share without starving out other candidates in
/// practice.
const FIND_PEER_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(4);

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
    // (`device_connection::commands::local_bluetooth_address`'s own env
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

/// One `LazyAndroidBackend` for the process's lifetime. Building the wrapper
/// itself touches neither `ndk-context` nor `AndroidBackend::new()` — that
/// work is deferred by `LazyAndroidBackend` to its first real use (the first
/// `capabilities`/`scan`/`connect`/`advertise` call), which by construction
/// only happens from a genuine post-startup JS -> Rust command (see
/// `start_peripheral_once` and `spawn_dial_loop`'s callers), never from
/// `.setup()`. See `android_lazy`'s module doc for why an eager attempt
/// there would crash.
#[cfg(target_os = "android")]
async fn backend() -> Result<Arc<dyn Backend>, String> {
    static BACKEND: OnceCell<Arc<dyn Backend>> = OnceCell::const_new();
    let backend = BACKEND
        .get_or_init(|| async { Arc::new(android_lazy::LazyAndroidBackend::new()) as Arc<dyn Backend> })
        .await;
    Ok(Arc::clone(backend))
}

/// Starts the Bluetooth peripheral acceptor loop exactly once. Android-only:
/// on Linux `lib.rs` spawns `run_server` unconditionally from `.setup()`,
/// which is safe there since `LinuxBackend::new()` has no Android-context
/// ordering requirement. On Android that same eager spawn would race
/// `tao`'s own context bring-up (see `android_lazy`'s module doc), so the
/// first call instead comes from `space_sync_tick_impl` — a
/// `#[tauri::command]`, whose first real invocation can only happen once
/// the WebView/Activity has actually dispatched an IPC call, a strictly
/// later and safer point than anything obtainable from `.setup()` itself.
#[cfg(target_os = "android")]
pub fn start_peripheral_once(state: DeviceConnectionState, db_path: PathBuf) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        tauri::async_runtime::spawn(run_server(state, db_path));
    });
}

pub struct BleLink {
    channel: DatagramChannel,
    peer_addr: String,
}

impl BleLink {
    fn new(channel: DatagramChannel) -> Self {
        let peer_addr = channel.peer().0.clone();
        Self { channel, peer_addr }
    }
}

#[async_trait]
impl Link for BleLink {
    fn kind(&self) -> TransportKind {
        TransportKind::Bluetooth
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
        // included, and `find_peer_address` allows `FIND_PEER_CANDIDATE_TIMEOUT`
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
pub async fn dial(address: &str) -> Result<Box<dyn Link>, String> {
    let backend = backend().await?;
    let peer = PeerAddress(address.to_string());
    let channel = datagram::connect(backend, &peer, &datagram_config())
        .await
        .map_err(|err| format!("ble connect to {address} failed: {err}"))?;
    Ok(Box::new(BleLink::new(channel)))
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
/// Dials `address` and confirms it's genuinely `peer_id` via `BluetoothProbe`
/// (not `perform_client_auth`: the ordinary Auth path requires Bluetooth to
/// already be enabled for this pair, which is exactly the precondition
/// `find_peer_address` exists to help establish -- reusing it would mean
/// this discovery flow could never succeed for its actual target case).
///
/// Falls back to `perform_client_auth` if `BluetoothProbe` goes
/// unanswered: a peer still running a build from before that frame
/// existed can't decode it at all and just silently closes the
/// connection, indistinguishable here from "not paired." The ordinary
/// Auth path still works against such a peer *if* Bluetooth happens to
/// already be enabled for this pair (the one case its
/// `check_bluetooth_enabled` gate allows), recovering "Find via
/// Bluetooth" for the "already enabled, address changed" scenario even
/// against a peer that can't speak the newer discovery protocol. A
/// never-enabled pair against such a peer remains a genuine limit of
/// protocol evolution -- there's no discovery flow to fall back to that
/// doesn't equally require the peer to understand it.
///
/// `None` on any failure along the way; the caller is responsible for
/// bounding how long this (now up to two sequential dial+handshake
/// attempts) is allowed to run -- see `FIND_PEER_CANDIDATE_TIMEOUT`.
async fn probe_candidate(state: &DeviceConnectionState, address: &str, peer_id: &str) -> Option<()> {
    if let Ok(mut link) = dial(address).await {
        if send_frame(
            link.as_mut(),
            &PeerFrame::BluetoothProbe {
                device_id: state.identity.device_id.clone(),
            },
        )
        .await
        .is_ok()
        {
            if let Some(Ok(PeerFrame::BluetoothProbeReply { device_id })) =
                recv_frame(link.as_mut()).await
            {
                if device_id == peer_id {
                    return Some(());
                }
            }
        }
    }

    let mut fallback_link = dial(address).await.ok()?;
    session::perform_client_auth(fallback_link.as_mut(), &state.identity.device_id, peer_id)
        .await
        .ok()
        .map(|_protocol_version| ())
}

pub async fn find_peer_address(
    state: DeviceConnectionState, db_path: PathBuf, peer_id: String, timeout: Duration,
) -> Result<Option<String>, String> {
    use futures_util::StreamExt;

    let backend = backend().await?;
    let mut discovered = backend
        .scan(datagram_config().service)
        .await
        .map_err(|err| format!("ble scan failed: {err}"))?;

    let mut tried: HashSet<String> = HashSet::new();
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let candidate = match tokio::time::timeout(remaining, discovered.next()).await {
            Ok(Some(Ok(candidate))) => candidate,
            // A backend-level scan failure (e.g. Android's async
            // `onScanFailed` for an adapter, registration, or permission
            // problem) means Bluetooth itself is unusable right now, not
            // merely "no candidate seen yet" -- surface it as an error so
            // the caller doesn't report a misleading "not found".
            Ok(Some(Err(err))) => return Err(format!("ble scan failed: {err}")),
            // Timed out, or the stream ended with nothing left to poll:
            // both are a genuine "not found within the deadline".
            Ok(None) | Err(_) => return Ok(None),
        };
        let address = candidate.address.0;
        if !tried.insert(address.clone()) {
            continue;
        }
        // The dial+probe+reply round trip is bounded by the *remaining*
        // scan deadline too, not left unbounded -- a candidate that
        // accepts the connection but never answers `BluetoothProbe` would
        // otherwise leave `recv_frame` waiting indefinitely, well past the
        // `timeout` this function promises its caller (and the "Find via
        // Bluetooth" button's advertised bound).
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let confirmed = tokio::time::timeout(
            remaining.min(FIND_PEER_CANDIDATE_TIMEOUT),
            probe_candidate(&state, &address, &peer_id),
        )
        .await
        .ok()
        .flatten()
        .is_some();
        if confirmed {
            let db_path = db_path.clone();
            let peer_id = peer_id.clone();
            let address_owned = address.clone();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db_path);
                crate::services::device_connection::persist_bluetooth_address_and_maybe_enable(
                    &mut conn, &peer_id, &address_owned,
                )
            })?;
            return Ok(Some(address));
        }
    }
}

/// A nearby, not-yet-paired device discovered via BLE while both sides are
/// in add-mode — the Bluetooth-side entry `AddDeviceView.vue`'s unified
/// candidate list merges alongside mDNS-discovered ones (ADR 0002 Phase 3).
pub struct AddModeCandidate {
    pub address: String,
    pub device_id: String,
    pub hostname: String,
}

/// Dials `address` and exchanges `DiscoveryHello`/`DiscoveryHelloReply`.
/// `None` on any failure along the way (dial, send, no/wrong reply); the
/// caller is responsible for bounding how long this is allowed to run.
async fn probe_discovery_hello(address: &str) -> Option<PeerFrame> {
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
/// (unlike `find_peer_address`, this feeds a picker list, not a single
/// confirm-and-persist action) — callers needing an ongoing view call this
/// repeatedly rather than once for a long window.
pub async fn scan_add_mode_candidates(
    my_device_id: &str, timeout: Duration,
) -> Result<Vec<AddModeCandidate>, String> {
    use futures_util::StreamExt;

    let backend = backend().await?;
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
        let mut discovered = backend
            .scan(datagram_config().service)
            .await
            .map_err(|err| format!("ble scan failed: {err}"))?;

        let mut flagged: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let peer = match tokio::time::timeout(remaining, discovered.next()).await {
                Ok(Some(Ok(peer))) => peer,
                // A backend-level scan failure (e.g. Android's async
                // `onScanFailed`) means Bluetooth itself is unusable, not
                // merely "no more candidates" -- propagate it like
                // `find_peer_address` does, rather than reporting an
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
            let _ = ADD_MODE_FLAG_BYTE;
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
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let reply = tokio::time::timeout(
            remaining.min(CANDIDATE_PROBE_TIMEOUT),
            probe_discovery_hello(&address),
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
/// `transport::tcp_ws::TcpWsTransport` for why production dial loops call
/// `dial()` directly rather than through this trait object.
#[allow(dead_code)]
pub struct BleTransport;

#[async_trait]
impl Transport for BleTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Bluetooth
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
/// only, matching `tcp_ws::run_server`/`sim::run_server` — `cli-plane` dials
/// out but does not run an inbound acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_server(state: DeviceConnectionState, db_path: PathBuf) {
    use futures_util::StreamExt;

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

    loop {
        let backend = match backend().await {
            Ok(backend) => backend,
            Err(err) => {
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
            Ok(stream) => stream,
            Err(err) => {
                log::warn!("[transport][ble] advertise failed, retrying in {delay:?}: {err}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
                continue;
            }
        };
        log::info!("[transport][ble] advertising, awaiting centrals");
        delay = Duration::from_secs(2);

        let mut restarting_for_add_mode_change = false;
        loop {
            tokio::select! {
                channel = incoming.next() => {
                    let Some(channel) = channel else { break; };
                    let link: Box<dyn Link> = Box::new(BleLink::new(channel));
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
                    tokio::spawn(session::run_peer_gate(link, state, db_path));
                }
                _ = add_mode_rx.changed() => {
                    log::info!("[transport][ble] add-mode changed; re-advertising");
                    restarting_for_add_mode_change = true;
                    break;
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

/// Dial loop: for every paired, Bluetooth-enabled, OS-paired peer with no
/// active session on this transport, attempt a central-role connect.
/// `candidates` is (peer_device_id, bluetooth_address) for peers meeting
/// those stored-metadata conditions — gathered by the caller from
/// `paired_devices` (see `device_connection::commands::
/// bluetooth_dial_candidates`), since unlike `tcp_ws`/`sim` there is no
/// presence worker or static port list to draw from here. ADR-0003
/// revision: dials unconditionally now, independent of Network's own state
/// or the pin -- see `tcp_ws::spawn_dial_loop`'s own doc comment.
pub fn spawn_dial_loop(state: &DeviceConnectionState, db_path: PathBuf, candidates: &[String]) {
    // While the user is adding a device, the dial loop gives up the adapter.
    //
    // One adapter supports one discovery session, and this loop holds one
    // for 8s out of every scan period. `scan_add_mode_candidates` opening a
    // second concurrent scan does not fail -- it returns a stream that
    // simply never yields, which is why Add Device found nothing while this
    // loop was finding the same phone every few seconds. Measured directly:
    // `add-mode scan saw 0 advertiser(s)` repeating once per poll, against
    // `scan: discovered ...` from this loop in the same log at the same time.
    //
    // Yielding is the right way round. Pairing is a short window the user is
    // actively waiting on; reconnecting an existing peer is background work
    // that loses nothing by pausing for it, and resumes on the next tick.
    if *add_mode_sender().borrow() {
        return;
    }

    let my_id = state.identity.device_id.clone();

    for peer_id in candidates {
        if !should_dial_peer(&my_id, peer_id, state.has_session_on(peer_id, TransportKind::Bluetooth)) {
            continue;
        }
        if is_backing_off(&dial_backoff_until().lock().unwrap(), peer_id, Instant::now()) {
            continue;
        }
        if is_bluetooth_dial_exhausted(peer_id) {
            continue;
        }
        // One retry loop per peer, not one per tick: `space_sync_tick`
        // (and therefore this function) runs every few seconds from the
        // frontend, and `dial_with_backoff` itself already loops with
        // backoff until a session claims or the peer stops being
        // eligible. Without this guard, every tick while a peer stays
        // unreachable would spawn *another* concurrent retry loop for the
        // same peer on top of the ones already running — unbounded tasks
        // and increasingly concurrent connection attempts to one address.
        if !in_flight_dials().lock().unwrap().insert(peer_id.clone()) {
            continue;
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id = peer_id.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id.clone()).await;
            in_flight_dials().lock().unwrap().remove(&peer_id);
        });
    }
}

/// Peers with a `dial_with_backoff` task currently running. See the doc
/// comment on the guard in `spawn_dial_loop` for why this exists.
fn in_flight_dials() -> &'static StdMutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| StdMutex::new(HashSet::new()))
}

/// Peers whose Bluetooth dial should not be retried before this instant --
/// a P1 review finding: `dial_with_backoff` giving up terminally (a
/// rejection that won't resolve itself by retrying sooner, e.g. a pre-v3
/// peer's own sticky-single-session code rejecting our now-unconditional
/// Bluetooth attempt with "session already active on another transport"
/// while it already has a Network session with us) used to just clear
/// `in_flight_dials`' guard and let the very next `space_sync_tick`
/// (every few seconds) spawn a brand new attempt -- a continuous
/// connect/reject/disconnect cycle for as long as the peer stays on an
/// incompatible build (or, pre-existing and not new to this PR: for any
/// peer that outright doesn't recognize us as paired). Entries are never
/// removed once their deadline passes -- `spawn_dial_loop`'s own check
/// just stops treating them as blocking, so there's nothing to clean up.
fn dial_backoff_until() -> &'static StdMutex<HashMap<String, Instant>> {
    static BACKOFF: OnceLock<StdMutex<HashMap<String, Instant>>> = OnceLock::new();
    BACKOFF.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Pure decision `spawn_dial_loop` delegates to -- split out so it's
/// directly unit-testable with synthetic timestamps, the same way
/// `should_dial_peer` is: `ble::dial` talks to real Bluetooth hardware, so
/// an end-to-end integration test of the backoff (actually rejecting a
/// dial and observing the next tick skip it) isn't constructible in this
/// test environment.
fn is_backing_off(backoff: &HashMap<String, Instant>, peer_id: &str, now: Instant) -> bool {
    backoff.get(peer_id).is_some_and(|until| now < *until)
}

/// How long a terminal rejection suppresses further Bluetooth dial
/// attempts for that peer. Fixed, not exponential -- still a 20x reduction
/// in attempt frequency against `space_sync_tick`'s few-second cadence,
/// without needing to persist a growing failure count anywhere.
const DIAL_BACKOFF: Duration = Duration::from_secs(60);

/// How long one continuous streak of connect/auth attempts (see
/// `dial_with_backoff`'s `streak_deadline`) is allowed to keep retrying
/// automatically before giving up and requiring an explicit
/// `retry_bluetooth_dial` call to resume. Distinct from `DIAL_BACKOFF`: that
/// one is for a *terminal* rejection (the peer explicitly said no); this one
/// is for a link that never even gets far enough to be rejected -- it just
/// keeps connecting, sometimes completing GATT setup, and dying before an
/// `AuthOk`/`AuthFail` ever arrives. Matches the frontend's own
/// `CONNECTING_TIMEOUT_MS` "stuck" threshold in spirit (see
/// `DeviceView.vue`), but this one actually stops the background work
/// instead of only relabelling it -- an indefinite silent retry loop behind
/// an unchanging "Still connecting..." was the actual user complaint (real
/// device evidence, 2026-09-01: repeated connect/MTU/discover/subscribe
/// successes each followed by "connection closed before auth reply" a few
/// seconds to tens of seconds later, for minutes, with nothing in this
/// crate or `ble-gatt` ever calling `disconnect()` on any deadline that
/// would explain it -- a real, unresolved radio-layer instability, not a
/// bug in this loop's own timeout logic before this constant existed).
const AUTO_RETRY_WINDOW: Duration = Duration::from_secs(60);

/// Peers whose Bluetooth dial has stopped auto-retrying after
/// `AUTO_RETRY_WINDOW` of no successful auth. Cleared only by
/// `retry_bluetooth_dial` (an explicit user action) or by Bluetooth being
/// disabled and re-enabled for the pair (`device_connection_commands`'s
/// enable path) -- deliberately *not* on any timer, matching the "wait for
/// the user to ask again" behavior that's the whole point of giving up in
/// the first place.
fn dial_exhausted() -> &'static StdMutex<HashSet<String>> {
    static EXHAUSTED: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    EXHAUSTED.get_or_init(|| StdMutex::new(HashSet::new()))
}

/// Whether `peer_id`'s Bluetooth dial is currently paused after exhausting
/// `AUTO_RETRY_WINDOW` -- consulted when building this peer's
/// `TransportStatusCode` so the row can show a distinct, honest state
/// instead of an indefinite "Connecting...".
pub fn is_bluetooth_dial_exhausted(peer_id: &str) -> bool {
    dial_exhausted().lock().unwrap().contains(peer_id)
}

/// Called from `DeviceConnectionState::try_claim_session` whenever a
/// Bluetooth session is actually claimed, regardless of which side dialed --
/// a P1 review finding on top of `check_accepting_side_exhaustion`'s own
/// session-branch: that only runs on the next `space_sync_tick` (a few
/// seconds out), so a session that authenticates and then drops again
/// *within* one tick interval was never observed as connected by the poll,
/// leaving a stale `dial_exhausted` entry (and `accepting_side_
/// unconnected_since` timestamp) in place indefinitely -- `spawn_dial_loop`
/// then kept skipping the peer even after the poll eventually ran. This is
/// the same clearing `check_accepting_side_exhaustion` already does, just
/// triggered by the actual claim event instead of waiting for a poll to
/// observe it.
pub fn note_bluetooth_session_claimed(peer_id: &str) {
    dial_exhausted().lock().unwrap().remove(peer_id);
    accepting_side_unconnected_since().lock().unwrap().remove(peer_id);
}

/// Drops every per-peer Bluetooth dial tracker this file keeps in process
/// memory, keyed by `peer_id` -- called on unpair and on a fresh pairing
/// (see call sites in `device_connection::commands`). A P2 review finding:
/// these are process-global maps/sets, so a peer that exhausted, got
/// unpaired, and was paired again under the same `peer_device_id` without an
/// app restart used to come back already exhausted -- `spawn_dial_loop`
/// would see the stale flag and skip it, and the row would report exhausted
/// immediately instead of getting a fresh `AUTO_RETRY_WINDOW`.
/// `paired_devices` itself has no such staleness problem
/// (`device_connection_unpair_impl` deletes the row outright), only this
/// crate's own in-memory trackers do. Broader than what `retry_bluetooth_
/// dial` itself clears (that function's doc comment covers just the two
/// give-up trackers a retry click is meant to reset): this also drops
/// `dial_backoff_until`, a *terminal-rejection* backoff that's equally
/// stale-by-`peer_device_id` across an unpair/re-pair, but that an ordinary
/// retry click was never meant to bypass.
pub fn clear_dial_exhaustion(peer_id: &str) {
    dial_exhausted().lock().unwrap().remove(peer_id);
    accepting_side_unconnected_since().lock().unwrap().remove(peer_id);
    dial_backoff_until().lock().unwrap().remove(peer_id);
}

/// Resume Bluetooth dial retries for `peer_id` after
/// `is_bluetooth_dial_exhausted` -- the backend half of the Device page's
/// "click the row to try again" affordance. A no-op if the peer is already
/// connected. Clears both give-up trackers synchronously, then spawns a
/// one-off `dial_with_backoff` streak for `peer_id` directly -- *not* gated
/// by `should_dial_peer`.
///
/// That bypass is the actual fix for a P1 review finding: on the accepting
/// side of a pair (`my_id > peer_id`), `should_dial_peer` is false forever
/// -- it's not a fact that changes -- so `spawn_dial_loop` would never spawn
/// anything for this peer no matter how many ticks pass. Before this, retry
/// on that side only cleared the flags above and relied on the *next*
/// `spawn_dial_loop` tick to actually dial, which never came; the row would
/// just sit on "Connecting..." for another `AUTO_RETRY_WINDOW` and land back
/// on exhausted with nothing having attempted a connection. `dial_with_backoff`
/// itself never consults `should_dial_peer` -- it's agnostic about which side
/// is "supposed" to dial -- so spawning it directly here works for both
/// sides identically. `in_flight_dials`'s existing guard is what keeps this
/// safe if an automatic streak (dialing side) or an earlier retry click is
/// already running for the same peer: the `insert` below just fails and this
/// becomes a no-op instead of a second concurrent attempt, and
/// `try_claim_session` inside `dial_with_backoff`/`run_session` is the
/// existing collision-safety net if both sides' connections happen to
/// complete auth around the same time.
///
/// Deliberately takes no `candidates` list -- two P2 review findings on
/// earlier revisions of this function both had the same root cause: a
/// caller reading the peer's dial address via its own DB connection first
/// (once the shared, Mutex-guarded `AppDbConnection` used by every other
/// Tauri command), so the OS bond check inside that lookup (a `bluetoothctl`
/// subprocess call, up to 5s) ran while that shared lock was held. Looking
/// the address up here instead, on the spawned task, with its own fresh
/// connection via `open_db_at_path` -- exactly `is_still_bluetooth_eligible`'s
/// own established pattern -- means neither caller's lock is ever touched by
/// this at all.
pub fn retry_bluetooth_dial(state: &DeviceConnectionState, peer_id: &str) {
    dial_exhausted().lock().unwrap().remove(peer_id);
    // Also give the accepting-side tracker (below) a fresh window: without
    // this, a peer we never dial ourselves (we're the accepting side for
    // it) would just get marked exhausted again the moment the spawned
    // attempt below fails, since its "unconnected since" clock would still
    // read the distant past.
    accepting_side_unconnected_since().lock().unwrap().remove(peer_id);

    if state.has_session_on(peer_id, TransportKind::Bluetooth) {
        return;
    }
    if !in_flight_dials().lock().unwrap().insert(peer_id.to_string()) {
        return;
    }

    let db_path = state.db_path.clone();
    let state = state.clone();
    let peer_id = peer_id.to_string();
    tauri::async_runtime::spawn(async move {
        if !is_still_bluetooth_eligible(&db_path, &peer_id) {
            in_flight_dials().lock().unwrap().remove(&peer_id);
            return;
        }
        dial_with_backoff(state, db_path, peer_id.clone()).await;
        in_flight_dials().lock().unwrap().remove(&peer_id);
    });
}

/// Per-peer timestamp of the first tick this process observed itself as
/// the *accepting* side (`my_id > peer_id`, so `should_dial_peer` never
/// spawns a `dial_with_backoff` for it) with no Bluetooth session claimed.
/// A P1 review finding: `AUTO_RETRY_WINDOW`/`streak_deadline` above only
/// exist on the dialing side -- without an equivalent here, the accepting
/// side had no way to ever stop reporting "Connecting..." even after its
/// peer legitimately gave up dialing and showed "Unavailable" on its own
/// screen, a real cross-device asymmetry (plausibly the exact gray-vs-
/// amber mismatch observed between two real paired devices in the field).
fn accepting_side_unconnected_since() -> &'static StdMutex<HashMap<String, Instant>> {
    static UNCONNECTED_SINCE: OnceLock<StdMutex<HashMap<String, Instant>>> = OnceLock::new();
    UNCONNECTED_SINCE.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Called every `space_sync_tick` alongside `spawn_dial_loop`, over the
/// same candidate list, for the peers *this* process never dials --
/// gives the passive/accepting side the same "give up after
/// `AUTO_RETRY_WINDOW`" reporting the dialing side already gets from
/// `dial_with_backoff`, without needing a new wire message: the accepting
/// side has no attempt of its own to bound, only the absence of a session
/// to time. Reuses `dial_exhausted`/`is_bluetooth_dial_exhausted` for the
/// actual reporting, so every downstream consumer (both status polls, the
/// frontend's retry affordance) already works identically for both roles.
pub fn check_accepting_side_exhaustion(state: &DeviceConnectionState, candidates: &[String]) {
    let my_id = &state.identity.device_id;
    let now = Instant::now();
    let mut unconnected_since = accepting_side_unconnected_since().lock().unwrap();
    for peer_id in candidates {
        if state.has_session_on(peer_id, TransportKind::Bluetooth) {
            unconnected_since.remove(peer_id);
            // A session existing at all is proof the link can work right
            // now, regardless of how it got established (the peer could
            // have dialed in while we still held a stale exhausted flag
            // from an earlier streak) -- if it drops again later, that's a
            // fresh streak and deserves its own full window, not an
            // immediate re-report of exhaustion left over from before.
            //
            // A P1 review finding: this check used to run *after* the
            // `my_id < peer_id` skip below, so it never fired at all on the
            // dialing side. `retry_bluetooth_dial`'s direct dial bypasses
            // `should_dial_peer`, so the *accepting* side can now be the one
            // that establishes a session -- leaving the dialing side's own
            // stale `dial_exhausted` flag (set by its own earlier
            // `dial_with_backoff` giving up) never cleared. When that
            // session later dropped, `spawn_dial_loop` kept honoring the
            // stale flag and refused to ever dial this peer again,
            // one-manual-retry-per-session forever. Checking every
            // candidate's session state before the dialer-role filter below
            // fixes that for both roles identically.
            dial_exhausted().lock().unwrap().remove(peer_id);
            continue;
        }
        if my_id < peer_id {
            // should_dial_peer's own rule: we dial this one ourselves, so
            // dial_with_backoff's streak_deadline already covers the
            // no-session case.
            continue;
        }
        let since = *unconnected_since.entry(peer_id.clone()).or_insert(now);
        if now.duration_since(since) >= AUTO_RETRY_WINDOW {
            dial_exhausted().lock().unwrap().insert(peer_id.clone());
        }
    }
}

/// Deterministic dialer rule, mirroring `tcp_ws::should_dial_peer`/
/// `sim::should_dial_fallback_peer`: exactly one side of a pair ever
/// attempts to dial, so both peers dialling each other in the same tick
/// can't race to claim the same session on both ends.
fn should_dial_peer(my_id: &str, peer_id: &str, has_session: bool) -> bool {
    my_id < peer_id && !has_session
}

/// Re-reads `paired_devices` for the current, live answer to "is this peer
/// still a valid Bluetooth dial target" — which since ADR-0006 means only
/// "is Bluetooth still enabled for this pair". `block_in_place` around the
/// blocking DB open, matching `space_sync::session::check_paired`'s existing
/// pattern for the same kind of call from inside an async loop.
fn is_still_bluetooth_eligible(db_path: &std::path::Path, peer_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        bluetooth_dial_candidates(&mut conn).iter().any(|candidate_id| candidate_id == peer_id)
    })
}

/// `block_in_place` wrapper around the diagnostic address write, matching
/// `is_still_bluetooth_eligible`'s pattern for a blocking DB call made from
/// inside an async loop.
fn note_observed_bluetooth_address(db_path: &std::path::Path, peer_id: &str, address: &str) {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        store_observed_bluetooth_address(&mut conn, peer_id, address);
    })
}

/// How long one `connect_by_advertisement` pass listens for candidates
/// before giving up on this attempt. The *window*; `idle_scan_period`
/// below decides how often the window opens.
const DIAL_SCAN_WINDOW: Duration = Duration::from_secs(8);

/// Longest gap between scan windows while a peer is unreachable, when the
/// user is watching (ADR-0006 slice 3).
const SCAN_PERIOD_FOREGROUND: Duration = Duration::from_secs(30);

/// The same, for the background daemon. Double, because nobody is waiting
/// on the row and the phone is on battery.
const SCAN_PERIOD_BACKGROUND: Duration = Duration::from_secs(60);

/// How long to wait before opening the next scan window.
///
/// Duty-cycling at all is not a micro-optimisation: continuous scanning
/// destabilised the development desktop's adapter badly enough to drop the
/// machine's unrelated Bluetooth devices, which is recorded under ADR-0005's
/// method traps. On the phone it is also simply expensive -- and the phone
/// already runs Google's own continuous Nearby scanner, so ours is not the
/// only consumer of that radio.
///
/// The two periods are a deliberate first guess, not a tuned answer. Issue
/// #171 changes what a *connected* pair costs, and both halves spend one
/// battery budget, so the numbers should be settled together once that
/// lands. See ADR-0006's design review.
fn idle_scan_period() -> Duration {
    if crate::services::space_sync::commands::frontend_is_driving() {
        SCAN_PERIOD_FOREGROUND
    } else {
        SCAN_PERIOD_BACKGROUND
    }
}

/// How long one candidate gets for its own dial plus auth handshake.
///
/// Deliberately much larger than `FIND_PEER_CANDIDATE_TIMEOUT` (4s), which
/// bounds the *discovery* probe where giving up fast and moving on is the
/// right trade. Here we have already decided to talk to this peer, and a
/// real BLE connect on this hardware has been measured at ~28s on its own
/// (see `dial_with_backoff`). Bounded by the caller's remaining give-up
/// window regardless, so this never extends the advertised 60s.
///
/// Hardware evidence for splitting the two budgets at all: sharing one
/// deadline with the scan window above meant a candidate discovered 6s into
/// an 8s window got 2s to connect, and every dial was abandoned mid-connect
/// with "connect guard dropped".
const DIAL_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(30);

/// When each peer was last seen advertising a matching fingerprint.
///
/// ADR-0006 slice 4: this is what lets a transport row say "not nearby"
/// honestly instead of sitting on amber "connecting…" at a peer that is in
/// another building. It is only ever written from a fingerprint match, so
/// "seen" means "seen advertising *as this peer*", not merely "some Fini
/// device was in range".
fn last_seen_advertising() -> &'static StdMutex<HashMap<String, Instant>> {
    static LAST_SEEN: OnceLock<StdMutex<HashMap<String, Instant>>> = OnceLock::new();
    LAST_SEEN.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn note_peer_advertising(peer_id: &str) {
    if let Ok(mut seen) = last_seen_advertising().lock() {
        seen.insert(peer_id.to_string(), Instant::now());
    }
}

/// Whether `peer_id` has advertised recently enough to still count as
/// nearby.
///
/// The window is derived from the scan period rather than picked: it has to
/// span several scan cycles, because a single missed beacon is ordinary --
/// the listening window is 8s out of every 30s or 60s, so most of a peer's
/// advertisements are simply not heard. A window shorter than a few cycles
/// would flap the row between "nearby" and "not nearby" while nothing
/// changed.
///
/// The cost is that the row is honest but unhurried: after a peer really
/// leaves, it can take up to three minutes in the background to say so.
pub fn peer_seen_advertising_recently(peer_id: &str) -> bool {
    let freshness = idle_scan_period() * 3;
    match last_seen_advertising().lock() {
        Ok(seen) => seen.get(peer_id).is_some_and(|at| at.elapsed() < freshness),
        Err(_) => false,
    }
}

/// Finds `peer_id` in the air and returns an authenticated link to it.
///
/// This is ADR-0006's core move. There is no stored address to dial: Android
/// advertises under a rotating resolvable private address, so the only
/// reliable way to reach a peer is to connect to whoever is advertising
/// Fini's service UUID and let the app-level `Auth` frame say who answered.
/// `perform_client_auth` already rejects a peer whose `device_id` is not the
/// expected one, and `specs/device-connect/README.md` already names that
/// exchange — not the OS bond — as the trust boundary.
///
/// Every candidate costs a dial plus a handshake, so this is bounded twice:
/// by `budget` overall (the caller's remaining give-up window) and by
/// `DIAL_SCAN_WINDOW` on the listening itself. A later ADR-0006 slice adds
/// an advertised identity fingerprint, which turns this from "try each
/// advertiser" into "try the one whose fingerprint matches".
async fn connect_by_advertisement(
    state: &DeviceConnectionState, peer_id: &str, budget: Duration,
) -> AdvertisementDial {
    use futures_util::StreamExt;

    let mut last_auth_error = None;
    let Ok(backend) = backend().await else {
        return AdvertisementDial::NoneReachable { last_auth_error };
    };

    // Two deadlines, not one. `scan_deadline` bounds how long we listen for
    // candidates; `overall_deadline` bounds the whole pass on the caller's
    // behalf. Collapsing them means a candidate discovered late in the
    // listening window inherits only its leftovers as its connect budget,
    // which on hardware abandoned every dial 2s in.
    let started = tokio::time::Instant::now();
    let overall_deadline = started + budget;
    let wanted_fingerprint = fingerprint_of(peer_id);
    let mut tried: HashSet<String> = HashSet::new();

    loop {
        // Scan and dial are strictly sequential, never concurrent, and the
        // discovery stream is dropped before any dial begins.
        //
        // Hardware evidence for this shape: holding the stream open across
        // the dial made BlueZ answer `Connect` with nothing at all until
        // ble-gatt's own 20s timeout fired, every single time, on a peer
        // sitting at rssi -62. An adapter cannot usefully drive active
        // discovery and establish a connection at the same moment, so
        // scanning while dialling meant competing with ourselves.
        let scan_deadline = tokio::time::Instant::now() + DIAL_SCAN_WINDOW;
        let found = {
            let Ok(mut discovered) = backend.scan(datagram_config().service).await else {
                return AdvertisementDial::NoneReachable { last_auth_error };
            };
            let mut found = None;
            loop {
                let listen_remaining =
                    scan_deadline.saturating_duration_since(tokio::time::Instant::now());
                if listen_remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(listen_remaining, discovered.next()).await {
                    Ok(Some(Ok(candidate))) => {
                        let address = candidate.address.0;
                        // Only dial an advertiser whose fingerprint matches
                        // the peer we want. A missing fingerprint is *not*
                        // tolerated, and that is a deliberate reversal of
                        // this code's first version.
                        //
                        // The `Auth` frame carries our own `device_id` and
                        // the expected peer's in plaintext and proves
                        // nothing cryptographically (issue #162), so dialling
                        // an unknown advertiser hands both identifiers to
                        // whoever happens to be advertising Fini's service
                        // nearby. Tolerating a missing fingerprint for the
                        // sake of older builds would keep that door open
                        // permanently. Fini is in open alpha with a
                        // no-legacy policy, so the older build upgrades
                        // instead.
                        let advertised = advertised_fingerprint(
                            candidate
                                .manufacturer_data
                                .get(&FINI_MANUFACTURER_ID)
                                .map(|payload| payload.as_slice()),
                        );
                        if advertised != Some(wanted_fingerprint) {
                            continue;
                        }
                        // Recorded on the match, before the dial: the peer
                        // is provably in range whether or not connecting to
                        // it then succeeds, and those are different facts
                        // for the row to report.
                        note_peer_advertising(peer_id);
                        if tried.insert(address.clone()) {
                            found = Some(address);
                            break;
                        }
                    }
                    // A backend-level scan failure means Bluetooth itself is
                    // unusable right now; either way this pass is over, and
                    // the caller's backoff decides what happens next.
                    Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                }
            }
            found
            // `discovered` is dropped here, stopping discovery, before the
            // dial below runs.
        };

        let Some(address) = found else {
            return AdvertisementDial::NoneReachable { last_auth_error };
        };

        let budget_left = overall_deadline.saturating_duration_since(tokio::time::Instant::now());
        if budget_left.is_zero() {
            return AdvertisementDial::NoneReachable { last_auth_error };
        }
        // Bounded per candidate as well as overall: an advertiser that
        // accepts the connection but never answers `Auth` would otherwise
        // hold the whole window on its own. `dial` and `perform_client_auth`
        // each wait unboundedly by themselves -- see `dial_with_backoff`'s
        // note on real-device evidence of a single `connect()` taking ~28s.
        let attempt = tokio::time::timeout(budget_left.min(DIAL_CANDIDATE_TIMEOUT), async {
            let mut link = dial(&address).await?;
            let version =
                session::perform_client_auth(link.as_mut(), &state.identity.device_id, peer_id)
                    .await?;
            Ok((link, version))
        })
        .await;

        match attempt {
            Ok(Ok((link, version))) => {
                return AdvertisementDial::Connected {
                    link,
                    protocol_version: version,
                    address,
                }
            }
            // Logged at info, not debug: this is the load-bearing path while
            // ADR-0006 is being brought up, and a silent candidate failure
            // is indistinguishable from "nothing was advertising" in a
            // hardware log -- which already cost one debugging round.
            Ok(Err(err)) => {
                log::info!("[transport][ble] candidate {address} is not {peer_id}: {err}");
                last_auth_error = Some(err);
            }
            Err(_elapsed) => {
                log::info!(
                    "[transport][ble] candidate {address} did not finish connect+auth in time"
                );
            }
        }
    }
}

/// What one `connect_by_advertisement` pass found.
enum AdvertisementDial {
    Connected {
        link: Box<dyn Link>,
        protocol_version: u32,
        address: String,
    },
    /// Nothing in the air authenticated as this peer. `last_auth_error`
    /// carries the last candidate's failure, if any candidate was tried at
    /// all. The caller reads it the same way the old address-based path read
    /// its auth result: an `auth rejected` prefix is a real peer refusing us
    /// and earns a backoff, anything else is ordinary radio noise.
    NoneReachable { last_auth_error: Option<String> },
}

async fn dial_with_backoff(state: DeviceConnectionState, db_path: PathBuf, peer_id: String) {
    let mut delay = Duration::from_secs(2);
    // Real-device evidence (2026-09-01, actual BLE hardware, not the mock
    // radio): a flaky link can keep connecting, negotiating MTU, and even
    // completing GATT service discovery, then dying before the app-level
    // Auth reply arrives -- repeatedly, for minutes, with no code-level
    // timeout anywhere in this loop or `perform_client_auth` ever calling it
    // off. Every one of those failed attempts left the UI showing an
    // unchanging, indefinite "Still connecting..." with no way to tell
    // whether it was making progress or stuck. Cap how long one continuous
    // unsuccessful streak auto-retries before giving up and surfacing that
    // giving-up as a distinct, visible state (`retry_bluetooth_dial`'s doc
    // comment) instead of retrying forever in the background.
    let mut streak_deadline = tokio::time::Instant::now() + AUTO_RETRY_WINDOW;

    loop {
        if state.has_session_on(&peer_id, TransportKind::Bluetooth) {
            return;
        }
        // Re-checked every retry, not just at the moment this task was
        // spawned: the candidate list `spawn_dial_loop` built is a single
        // snapshot, so a peer reachable only later in this backoff loop
        // would otherwise still complete auth and claim a session even
        // after the user disabled Bluetooth for them or unpaired them
        // entirely in the meantime — a setting meant to stop future
        // Bluetooth use silently not taking effect.
        if !is_still_bluetooth_eligible(&db_path, &peer_id) {
            log::info!(
                "[transport][ble] {peer_id} is no longer Bluetooth-enabled; stopping dial retries"
            );
            return;
        }
        // The same adapter hand-off `spawn_dial_loop` performs, for a task
        // that was already running when add-mode began. Returning rather
        // than sleeping: the next `space_sync_tick` after add-mode ends
        // spawns a fresh attempt, and holding a task open across an
        // arbitrarily long pairing session would keep this peer's
        // `in_flight_dials` slot occupied for no benefit.
        if *add_mode_sender().borrow() {
            log::info!("[transport][ble] pausing dial to {peer_id}: add-mode is using the adapter");
            return;
        }
        if tokio::time::Instant::now() >= streak_deadline {
            // A P1 review finding: a session for this peer can be claimed
            // elsewhere (an inbound connection this process accepts, or the
            // peer's own direct retry dial) while `is_still_bluetooth_
            // eligible` above was still scanning -- on Linux that scan can
            // take up to 5s per Bluetooth-enabled peer. A claim landing in
            // that window calls `note_bluetooth_session_claimed`, clearing
            // `dial_exhausted`; without this recheck, this branch would
            // then blindly write it right back while the just-recovered
            // session is still live. Re-checking `has_session_on`
            // immediately before the write, not just at the top of the
            // loop, closes that TOCTOU window.
            if state.has_session_on(&peer_id, TransportKind::Bluetooth) {
                return;
            }
            log::info!(
                "[transport][ble] {peer_id} did not complete auth within {AUTO_RETRY_WINDOW:?}; \
                 pausing automatic retries until the user asks again"
            );
            dial_exhausted().lock().unwrap().insert(peer_id.clone());
            return;
        }

        // The top-of-loop deadline check above only fires between
        // attempts -- neither `dial` nor `perform_client_auth` has any
        // timeout of its own (confirmed: `ble_gatt::backend::android::
        // connect` waits unboundedly on its callback channel, and
        // `perform_client_auth` waits unboundedly on `recv_frame`), so a
        // single slow or hung attempt could keep this loop from ever
        // reaching that check again -- a P1 review finding: real-device
        // evidence already showed one raw `connect()` alone take ~28s
        // against the same 60s budget this is supposed to bound. Bound
        // each attempt by whatever's left of `streak_deadline`, not a
        // separate fixed timeout, so the advertised total give-up window
        // holds regardless of how that time gets spent.
        let attempt_budget = streak_deadline.saturating_duration_since(tokio::time::Instant::now());
        match connect_by_advertisement(&state, &peer_id, attempt_budget).await {
            AdvertisementDial::Connected {
                link,
                protocol_version,
                address,
            } => {
                log::info!("[transport][ble] auth OK with {peer_id} via {address}");
                // The scan+connect+auth round trip is real wall-clock time
                // during which the user could disable Bluetooth or unpair
                // entirely; without this, a disable/unpair that lands in
                // that window would still be raced by a session claim that
                // started before it. Re-run the same eligibility check the
                // top of this loop uses, right before claiming, to close
                // that window.
                if !is_still_bluetooth_eligible(&db_path, &peer_id) {
                    log::info!(
                        "[transport][ble] {peer_id} became ineligible during the \
                         connect/auth handshake; discarding this session"
                    );
                    return;
                }
                // Diagnostics only. Since ADR-0006 nothing dials this
                // address -- it records "where we last saw this peer", which
                // is what keeps a hardware log readable after the fact.
                note_observed_bluetooth_address(&db_path, &peer_id, &address);
                let (tx, rx) = tokio::sync::mpsc::channel(64);
                if state.try_claim_session(&peer_id, TransportKind::Bluetooth, tx, &db_path) {
                    session::run_session(
                        link,
                        rx,
                        state.clone(),
                        db_path.clone(),
                        peer_id.clone(),
                        protocol_version,
                    )
                    .await;
                    log::info!("[transport][ble] session with {peer_id} ended");
                }
                // Auth actually succeeding is real forward progress,
                // independent of how long the eventual session lasted
                // -- give the next streak (if the session later
                // drops) its own full patience window rather than
                // carrying over elapsed time from before this proof
                // the link *can* work.
                delay = Duration::from_secs(2);
                streak_deadline = tokio::time::Instant::now() + AUTO_RETRY_WINDOW;
            }
            AdvertisementDial::NoneReachable {
                last_auth_error: Some(err),
            } => {
                log::warn!("[transport][ble] no candidate authenticated as {peer_id}: {err}");
                // "bluetooth disabled" is not permanent the way "unknown
                // device" is -- the user could re-enable at any time. Keep
                // retrying rather than giving up outright. The old "not
                // currently OS-paired" rejection is gone entirely: ADR-0006
                // removed the gate that produced it.
                if err.starts_with("auth rejected")
                    && !err.contains("bluetooth disabled for this pair")
                {
                    // Not just "don't retry within this task" --
                    // `spawn_dial_loop` would otherwise spawn a
                    // fresh one on the very next tick regardless.
                    // See `dial_backoff_until`'s own doc comment.
                    dial_backoff_until()
                        .lock()
                        .unwrap()
                        .insert(peer_id.clone(), Instant::now() + DIAL_BACKOFF);
                    return; // not paired; don't retry
                }
            }
            AdvertisementDial::NoneReachable {
                last_auth_error: None,
            } => {
                // Nothing in the air answered as this peer within the
                // window. Ordinary: the peer is out of range, its radio is
                // off, or its advertisement simply did not land in this
                // listening window.
                log::info!("[transport][ble] {peer_id} did not answer in the air this round");
            }
        }

        // P2 review finding: this sleep is reached by the unsuccessful arms
        // above, which (unlike the old `Err(_elapsed)` arms) don't `continue`
        // past it, so a failure landing shortly before `streak_deadline`
        // could still sleep the full (up to 30s) `delay` before the
        // top-of-loop check gets another chance to run, pushing the
        // advertised 60s give-up noticeably later. Bound it the same way the
        // attempt budget above is bounded, rather than duplicating a deadline
        // check into each unsuccessful arm individually.
        //
        // The ceiling is re-read every iteration rather than captured once:
        // the app can move between foreground and background during a single
        // unsuccessful streak, and the cadence should follow it rather than
        // stay on whatever it was when the streak began.
        let remaining_budget = streak_deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::time::sleep(delay.min(remaining_budget)).await;
        delay = (delay * 2).min(idle_scan_period());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_dial_only_from_the_lower_device_id_and_only_without_a_session() {
        assert!(should_dial_peer("local-a", "peer-b", false));
        assert!(!should_dial_peer("peer-b", "local-a", false));
        assert!(!should_dial_peer("local-a", "peer-b", true));
        assert!(!should_dial_peer("same-id", "same-id", false));
    }

    /// Regression test for a P1 review finding: a terminal auth rejection
    /// (e.g. a pre-v3 peer's own sticky-single-session code rejecting our
    /// now-unconditional Bluetooth dial) must suppress `spawn_dial_loop`
    /// from immediately spawning a fresh attempt against that peer on the
    /// very next tick, not just stop the one task that just failed.
    #[test]
    fn dial_backoff_suppresses_a_peer_until_its_deadline_passes() {
        let mut backoff = HashMap::new();
        let now = Instant::now();
        assert!(!is_backing_off(&backoff, "peer-a", now), "no entry yet -- must not back off");

        backoff.insert("peer-a".to_string(), now + Duration::from_secs(60));
        assert!(
            is_backing_off(&backoff, "peer-a", now),
            "within the backoff window -- must skip"
        );
        assert!(
            !is_backing_off(&backoff, "peer-b", now),
            "a different peer's entry must not affect this one"
        );
        assert!(
            !is_backing_off(&backoff, "peer-a", now + Duration::from_secs(61)),
            "once the deadline passes, the peer is eligible again"
        );
    }

    /// `add_mode_sender` is a process-global singleton (mirrors the real
    /// adapter's own single peripheral instance). `device_connection`'s
    /// `enter_add_mode_impl`/`leave_add_mode_impl` also flip it, so any test
    /// exercising those (see `transport::tests`) must hold
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
    /// in `device_connection::commands`.
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
}
