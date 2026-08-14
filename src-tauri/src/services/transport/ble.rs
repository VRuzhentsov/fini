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
//! Plays the same "Bluetooth fallback" role `transport::sim` plays for
//! tests/E2E, but for real: network is preferred (see
//! `transport::selection`), Bluetooth engages only when a paired peer is not
//! effectively reachable over the network. Unlike `tcp_ws` (backed by the
//! mDNS/UDP presence worker) and `sim` (statically configured ports), there
//! is no discovery step here — candidates come from stored per-peer
//! Bluetooth metadata (`paired_devices.bluetooth_address`, gated on
//! `bluetooth_enabled` and a live OS-pairing check), exactly what
//! `device_connection::commands::bluetooth_address_is_os_paired` already
//! checks for the enable command.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

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
                    eprintln!("[transport][ble] android backend construction failed: {err}");
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
use crate::services::device_connection::{bluetooth_dial_candidates, DeviceConnectionState};
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
    if *add_mode_sender().borrow() {
        config.advertised_manufacturer_data.insert(FINI_MANUFACTURER_ID, vec![ADD_MODE_FLAG_BYTE]);
    }
    config
}

/// `0xFFFF` is the Bluetooth SIG's own reserved value for "manufacturer
/// specific data" used for testing and non-market purposes — the
/// appropriate choice for a private, unregistered app like Fini rather
/// than picking an arbitrary value that could collide with a real vendor's
/// company ID some other nearby scanner is specifically watching for.
const FINI_MANUFACTURER_ID: u16 = 0xFFFF;
/// The entire payload of that manufacturer data: whether this device is
/// currently in add-mode. One byte is deliberate — legacy advertisements
/// cap out at 31 bytes total, and the service UUID above already spends
/// most of that; see `GattServiceSpec::manufacturer_data`'s own doc
/// comment in ble-gatt.
const ADD_MODE_FLAG_BYTE: u8 = 0x01;

/// Per-candidate cap for a dial+probe+reply confirmation round trip
/// (`probe_candidate`/`probe_discovery_hello`), separate from the overall
/// scan deadline: without this, a single candidate that accepts the
/// connection but never replies could consume the *entire* remaining scan
/// budget, starving out every other candidate that might otherwise have
/// matched sooner -- including the actual peer being searched for.
/// Deliberately shorter than `AddDeviceView.vue`'s own per-pass scan
/// duration (`BLUETOOTH_SCAN_DURATION_MS`, currently 4s): a cap that isn't
/// *materially* shorter than a single pass is no cap at all in practice,
/// since `remaining.min(...)` just reduces to `remaining` every time.
const CANDIDATE_PROBE_TIMEOUT: Duration = Duration::from_millis(1_500);

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
        self.channel.send(payload).await.map_err(|err| err.to_string())
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
    let mut discovered = backend
        .scan(datagram_config().service)
        .await
        .map_err(|err| format!("ble scan failed: {err}"))?;

    let mut candidates = Vec::new();
    let mut tried: HashSet<String> = HashSet::new();
    let deadline = tokio::time::Instant::now() + timeout;

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
        if !tried.insert(address.clone()) {
            continue;
        }
        let flagged =
            peer.manufacturer_data.get(&FINI_MANUFACTURER_ID).map(|v| v.as_slice())
                == Some([ADD_MODE_FLAG_BYTE].as_slice());
        if !flagged {
            continue;
        }
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
                eprintln!("[transport][ble] adapter unavailable, retrying in {delay:?}: {err}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
                continue;
            }
        };
        if !backend.capabilities().await.peripheral {
            eprintln!(
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
                eprintln!("[transport][ble] advertise failed, retrying in {delay:?}: {err}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
                continue;
            }
        };
        eprintln!("[transport][ble] advertising, awaiting centrals");
        delay = Duration::from_secs(2);

        let mut restarting_for_add_mode_change = false;
        loop {
            tokio::select! {
                channel = incoming.next() => {
                    let Some(channel) = channel else { break; };
                    let link: Box<dyn Link> = Box::new(BleLink::new(channel));
                    let state = state.clone();
                    let db_path = db_path.clone();
                    tokio::spawn(session::run_peer_gate(link, state, db_path));
                }
                _ = add_mode_rx.changed() => {
                    eprintln!("[transport][ble] add-mode changed; re-advertising");
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
        eprintln!("[transport][ble] serve stream ended unexpectedly; retrying in {delay:?}");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

/// Fallback dial loop: for every paired, Bluetooth-enabled, OS-paired peer
/// with no active session and no effectively-available network, attempt a
/// central-role connect. `candidates` is (peer_device_id, bluetooth_address)
/// for peers meeting those stored-metadata conditions — gathered by the
/// caller from `paired_devices` (see
/// `device_connection::commands::bluetooth_dial_candidates`), since unlike
/// `tcp_ws`/`sim` there is no presence worker or static port list to draw
/// from here.
pub fn spawn_dial_loop(
    state: &DeviceConnectionState,
    db_path: PathBuf,
    candidates: &[(String, String)],
) {
    let my_id = state.identity.device_id.clone();

    for (peer_id, address) in candidates {
        if !should_dial_peer(&my_id, peer_id, state.has_session(peer_id)) {
            continue;
        }
        // Network-first: only engage when the peer isn't effectively
        // reachable over the network — mirrors `sim::spawn_fallback_dial_loop`'s
        // `sim_is_preferred` check, but for the real fallback role instead
        // of the test stand-in. ADR-0003 Phase 3: an explicit Bluetooth pin
        // overrides this too, matching `dial_with_backoff`'s own override
        // below -- without it here, a pin never gets this far: the retry
        // task that would apply the override is never even spawned.
        if state.network_effectively_available(peer_id) && !peer_prefers_bluetooth(&db_path, peer_id) {
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
        let address = address.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id.clone(), address).await;
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

/// Deterministic dialer rule, mirroring `tcp_ws::should_dial_peer`/
/// `sim::should_dial_fallback_peer`: exactly one side of a pair ever
/// attempts to dial, so both peers dialling each other in the same tick
/// can't race to claim the same session on both ends.
fn should_dial_peer(my_id: &str, peer_id: &str, has_session: bool) -> bool {
    my_id < peer_id && !has_session
}

/// Re-reads `paired_devices` for the current, live answer to "is this peer
/// still a valid Bluetooth dial target" — enabled, still holding this exact
/// address, and currently OS-paired. `block_in_place` around the blocking
/// DB open, matching `space_sync::session::check_paired`'s existing pattern
/// for the same kind of call from inside an async loop.
fn is_still_bluetooth_eligible(db_path: &std::path::Path, peer_id: &str, address: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        bluetooth_dial_candidates(&mut conn)
            .into_iter()
            .any(|(candidate_id, candidate_address)| {
                candidate_id == peer_id && candidate_address == address
            })
    })
}

/// ADR-0003 Phase 3: has the user explicitly pinned this pair to Bluetooth?
/// If so, `dial_with_backoff`'s normal "only engage once network is
/// unreachable" fallback gating is overridden -- an explicit pin means
/// dial regardless of network's own availability. Also consulted by
/// `spawn_dial_loop`'s own outer gate (see its doc comment) -- without
/// that, a Bluetooth pin never even gets a `dial_with_backoff` task spawned
/// to apply this override in the first place.
fn peer_prefers_bluetooth(db_path: &std::path::Path, peer_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        crate::services::device_connection::peer_transport_preference(&mut conn, peer_id).as_deref()
            == Some("bluetooth")
    })
}

/// ADR-0003 Phase 3: has the user explicitly pinned this pair to Network?
/// If so, `dial_with_backoff` must not fall back to Bluetooth just because
/// network is *momentarily* unreachable -- an explicit pin is sticky, and a
/// transient blip isn't consent to hand the session to the other transport.
/// The opposite of `peer_prefers_bluetooth`, not merely its negation: "no
/// preference at all" must keep the old automatic-fallback behavior, so
/// this only returns `true` for an explicit `"network"` pin.
fn peer_prefers_network(db_path: &std::path::Path, peer_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        crate::services::device_connection::peer_transport_preference(&mut conn, peer_id).as_deref()
            == Some("network")
    })
}

async fn dial_with_backoff(state: DeviceConnectionState, db_path: PathBuf, peer_id: String, address: String) {
    let mut delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(30);

    loop {
        if state.has_session(&peer_id) {
            return;
        }
        if peer_prefers_network(&db_path, &peer_id) {
            return;
        }
        if state.network_effectively_available(&peer_id) && !peer_prefers_bluetooth(&db_path, &peer_id) {
            return;
        }
        // Re-checked every retry, not just at the moment this task was
        // spawned: the candidate list `spawn_dial_loop` built is a single
        // snapshot, so a peer reachable only later in this backoff loop
        // would otherwise still complete auth and claim a session even
        // after the user disabled Bluetooth for them or unpaired them
        // entirely in the meantime — a setting meant to stop future
        // Bluetooth use silently not taking effect.
        if !is_still_bluetooth_eligible(&db_path, &peer_id, &address) {
            eprintln!(
                "[transport][ble] {peer_id} is no longer Bluetooth-enabled/OS-paired; \
                 stopping dial retries"
            );
            return;
        }

        match dial(&address).await {
            Ok(mut link) => {
                match session::perform_client_auth(link.as_mut(), &state.identity.device_id, &peer_id)
                    .await
                {
                    Ok(peer_protocol_version) => {
                        eprintln!("[transport][ble] auth OK with {peer_id} via {address}");
                        state.record_bluetooth_dial_success(&peer_id);
                        // The connect+auth round trip is real wall-clock time
                        // during which the user could disable Bluetooth or
                        // unpair entirely; without this, a disable/unpair
                        // that lands in that window would still be raced by
                        // a session claim that started before it. Re-run the
                        // same eligibility check the top of this loop uses,
                        // right before claiming, to close that window.
                        if !is_still_bluetooth_eligible(&db_path, &peer_id, &address) {
                            eprintln!(
                                "[transport][ble] {peer_id} became ineligible during the \
                                 connect/auth handshake; discarding this session"
                            );
                            return;
                        }
                        let (tx, rx) = tokio::sync::mpsc::channel(64);
                        if state.try_claim_session(&peer_id, TransportKind::Bluetooth, tx) {
                            session::run_session(
                                link,
                                rx,
                                state.clone(),
                                db_path.clone(),
                                peer_id.clone(),
                                peer_protocol_version,
                            )
                            .await;
                            eprintln!("[transport][ble] session with {peer_id} ended");
                        }
                        delay = Duration::from_secs(2);
                    }
                    Err(err) => {
                        eprintln!("[transport][ble] auth with {peer_id} failed: {err}");
                        if err.starts_with("auth rejected") {
                            return; // not paired; don't retry
                        }
                        // Connection-level failure (link dropped mid-auth,
                        // etc.), not a rejection -- counts toward "recently
                        // unreliable," mirroring tcp_ws::dial_with_backoff.
                        state.record_bluetooth_dial_failure(&peer_id);
                    }
                }
            }
            Err(err) => {
                eprintln!("[transport][ble] connect to {peer_id} ({address}) failed: {err}");
                state.record_bluetooth_dial_failure(&peer_id);
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
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

    /// `add_mode_sender` is a process-global singleton (mirrors the real
    /// adapter's own single peripheral instance). `device_connection`'s
    /// `enter_add_mode_impl`/`leave_add_mode_impl` also flip it, so any test
    /// exercising those (see `transport::tests`) must hold
    /// `ADD_MODE_TEST_LOCK` too, the same way other process-global test
    /// state in this crate is serialized.
    #[test]
    fn datagram_config_advertises_the_add_mode_flag_only_while_enabled() {
        let _guard = ADD_MODE_TEST_LOCK.lock().unwrap();
        set_add_mode(false);
        let disabled = datagram_config();
        assert!(
            !disabled.advertised_manufacturer_data.contains_key(&FINI_MANUFACTURER_ID),
            "must not advertise the add-mode flag while add-mode is off"
        );

        set_add_mode(true);
        let enabled = datagram_config();
        assert_eq!(
            enabled.advertised_manufacturer_data.get(&FINI_MANUFACTURER_ID),
            Some(&vec![ADD_MODE_FLAG_BYTE])
        );

        set_add_mode(false);
        let disabled_again = datagram_config();
        assert!(
            !disabled_again.advertised_manufacturer_data.contains_key(&FINI_MANUFACTURER_ID),
            "must stop advertising the flag once add-mode is left again"
        );
    }

    fn seeded_preference_db(preferred: Option<&str>) -> std::path::PathBuf {
        use crate::schema::paired_devices;
        use diesel::prelude::*;

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fini.db");
        let mut conn = open_db_at_path(&db_path);
        diesel::insert_into(paired_devices::table)
            .values((
                paired_devices::peer_device_id.eq("peer-a"),
                paired_devices::display_name.eq("Peer A"),
                paired_devices::paired_at.eq("2026-04-07T00:00:00Z"),
                paired_devices::last_seen_at.eq(Option::<String>::None),
                paired_devices::pair_state.eq("paired"),
                paired_devices::preferred_transport.eq(preferred),
            ))
            .execute(&mut conn)
            .expect("insert paired device");
        std::mem::forget(dir);
        db_path
    }

    /// Regression test for a P1 review finding on ADR-0003 Phase 3: a
    /// Bluetooth pin and a Network pin must be mutually exclusive and both
    /// distinct from "no preference" -- `spawn_dial_loop`'s outer gate and
    /// `dial_with_backoff`'s inner gate both depend on exactly this
    /// three-way distinction to apply the pin correctly in either
    /// direction.
    #[tokio::test(flavor = "multi_thread")]
    async fn peer_prefers_bluetooth_and_network_are_mutually_exclusive() {
        let no_preference = seeded_preference_db(None);
        assert!(!peer_prefers_bluetooth(&no_preference, "peer-a"));
        assert!(!peer_prefers_network(&no_preference, "peer-a"));

        let bluetooth_pinned = seeded_preference_db(Some("bluetooth"));
        assert!(peer_prefers_bluetooth(&bluetooth_pinned, "peer-a"));
        assert!(!peer_prefers_network(&bluetooth_pinned, "peer-a"));

        let network_pinned = seeded_preference_db(Some("network"));
        assert!(!peer_prefers_bluetooth(&network_pinned, "peer-a"));
        assert!(peer_prefers_network(&network_pinned, "peer-a"));
    }
}
