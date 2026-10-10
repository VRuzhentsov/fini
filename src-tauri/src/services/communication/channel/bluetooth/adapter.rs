//! The Bluetooth adapter: one backend per process, and whether it worked
//! the last time anything used it.
//!
//! Process-wide on purpose, unlike the per-state objects beside it: a
//! process drives one physical adapter, and `ble-gatt`'s backends own a
//! single D-Bus session (Linux) or Activity-bound plugin (Android) each.

use super::*;

/// One `LinuxBackend` for the process's lifetime. `ble_gatt::backend::linux::LinuxBackend::new()`
/// opens a BlueZ D-Bus session and requires a powered adapter; constructing
/// it lazily (on first dial/serve attempt) rather than at startup means a
/// machine with no/unpowered Bluetooth adapter never fails app startup over
/// a transport most sessions won't use.
#[cfg(target_os = "linux")]
pub(super) async fn backend() -> Result<Arc<dyn Backend>, String> {
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
pub(super) async fn backend() -> Result<Arc<dyn Backend>, String> {
    crate::services::ble_plugin::backend()
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

pub(super) fn note_adapter_reachable() {
    ADAPTER_HEALTH.store(ADAPTER_REACHABLE, Ordering::Relaxed);
}

pub(super) fn note_adapter_unreachable() {
    ADAPTER_HEALTH.store(ADAPTER_UNREACHABLE, Ordering::Relaxed);
}

/// Records what a refused scan says about the adapter. A busy adapter (a
/// discovery already running, which ble-gatt has tried to recover) is
/// there and working -- only something else refuses it as missing.
pub(super) fn note_scan_refused(err: &ble_gatt::BleError) -> bool {
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
pub async fn probe_adapter_available(radio: &RadioArbiter) -> bool {
    let deadline = tokio::time::Instant::now() + PROBE_WAIT_FOR_OTHER_SCAN;
    loop {
        if radio.scan_is_running() {
            note_adapter_reachable();
            return true;
        }
        if let Some(_scan) = radio.try_scan_lease() {
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
        let radio = RadioArbiter::default();
        let _other_scan = radio.scan_lease().await;
        let _running = radio.start_running_scan();
        note_adapter_unreachable();

        assert!(
            probe_adapter_available(&radio).await,
            "a scan in flight means the adapter accepted a discovery session"
        );
        assert!(!is_bluetooth_adapter_unavailable());
    }

    /// Holding the lease is not a running scan. The add-mode scan takes the
    /// lease and then waits on the adapter; a probe in that window used to
    /// report a missing radio as working, so adding Bluetooth never failed.
    #[tokio::test(start_paused = true)]
    async fn adapter_probe_does_not_trust_a_scan_that_has_not_started() {
        let radio = Arc::new(RadioArbiter::default());
        let other_scan = radio.scan_lease().await;
        note_adapter_unreachable();

        let probe = tokio::spawn({
            let radio = radio.clone();
            async move { probe_adapter_available(&radio).await }
        });
        tokio::time::sleep(PROBE_WAIT_FOR_OTHER_SCAN + Duration::from_secs(1)).await;
        drop(other_scan);

        assert!(!probe.await.unwrap(), "no scan ever started, and the adapter was last seen unreachable");
    }
}
