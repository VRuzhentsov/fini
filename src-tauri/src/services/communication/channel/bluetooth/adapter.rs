//! The Bluetooth adapter: one per process, and whether it worked the last
//! time anything used it.
//!
//! Process-wide on purpose, unlike the per-state objects beside it: a
//! process drives one physical adapter, and each `ble-gatt` platform driver
//! owns one D-Bus session (Linux) or one Activity-bound plugin (Android).

use super::*;

/// The Bluetooth adapter of this device: the ble-gatt `Adapter`, made one
/// time for the process at the first use; whether it worked the last time
/// anything used it; and, in debug builds, the test switch over its driver.
pub struct BluetoothAdapter {
    adapter: tokio::sync::OnceCell<Adapter>,
    /// See `ADAPTER_UNKNOWN`.
    health: AtomicU8,
    #[cfg(feature = "devtools")]
    switch: std::sync::OnceLock<Arc<test_switch::RadioSwitch>>,
}

static THIS_DEVICE: BluetoothAdapter = BluetoothAdapter {
    adapter: tokio::sync::OnceCell::const_new(),
    health: AtomicU8::new(ADAPTER_UNKNOWN),
    #[cfg(feature = "devtools")]
    switch: std::sync::OnceLock::new(),
};

impl BluetoothAdapter {
    /// The one adapter of this process.
    pub fn this_device() -> &'static Self {
        &THIS_DEVICE
    }

    /// The ble-gatt adapter, made at the first use.
    ///
    /// The Linux driver opens a BlueZ D-Bus session and needs a powered
    /// adapter. Fini makes it at the first use, not at startup, so a
    /// computer without a usable adapter still starts the app.
    #[cfg(target_os = "linux")]
    pub(super) async fn adapter(&self) -> Result<Adapter, String> {
    // `get_or_try_init`, not `get_or_init` over a cached `Result`: the
    // latter permanently bakes in whatever the *first* call returned,
    // success or failure. If Fini starts while Bluetooth is off, BlueZ is
    // restarting, or the adapter is briefly unavailable, that first
    // failure would be replayed to every later dial/serve attempt for the
    // rest of the process's life, even after the adapter comes back —
    // `get_or_try_init` instead leaves the cell empty on error, so the
    // next call genuinely retries construction.
    // Bounded: `LinuxDriver::new()` opens a D-Bus system-bus session and
    // talks to BlueZ over it. On a machine with no `bluetoothd` running at
    // all (headless CI containers, minimal installs) there's nothing wrong
    // locally, but D-Bus's own service-activation/name-owner-wait semantics
    // can leave that call pending far longer than this transport is worth
    // blocking anything on — this runs at app startup via
    // `tauri::async_runtime::spawn`, so an unbounded wait here would tie up
    // an async worker thread for however long that takes. A timeout turns
    // "no adapter reachable" into a fast, retryable failure instead.
        const ADAPTER_INIT_TIMEOUT: Duration = Duration::from_secs(5);

        self.adapter
            .get_or_try_init(|| async {
                #[cfg(feature = "devtools")]
                if let Ok(endpoint) = std::env::var("FINI_BLE_MOCK_BROKER") {
                    return Ok(Adapter::new(self.switched(mock_driver(&endpoint).await?)).await);
                }
                match tokio::time::timeout(ADAPTER_INIT_TIMEOUT, LinuxDriver::new()).await {
                    Ok(Ok(driver)) => Ok(Adapter::new(self.switched(Arc::new(driver))).await),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(_) => Err(format!(
                        "timed out after {ADAPTER_INIT_TIMEOUT:?} waiting for the Bluetooth adapter"
                    )),
                }
            })
            .await
            .cloned()
    }

    /// The ble-gatt adapter on Android, which `tauri-plugin-ble-gatt` makes.
    /// Made at the first use, not at `.setup()`: the driver needs the
    /// Activity, and the Activity does not exist at `.setup()` (see the
    /// plugin's `lazy` module). The first use comes from a command or a tick
    /// after startup (`start_peripheral_once`, `start_exchange`). A
    /// `devtools` build puts the adapter's driver behind the test switch.
    #[cfg(target_os = "android")]
    pub(super) async fn adapter(&self) -> Result<Adapter, String> {
        self.adapter
            .get_or_try_init(|| async {
                let adapter = crate::services::ble_plugin::adapter().await?;
                #[cfg(feature = "devtools")]
                return Ok(Adapter::new(self.switched(adapter.driver())).await);
                #[cfg(not(feature = "devtools"))]
                Ok(adapter)
            })
            .await
            .cloned()
    }

    /// The driver as the adapter gets it. In a build without `devtools` this
    /// is the platform driver itself.
    #[cfg(all(target_os = "linux", not(feature = "devtools")))]
    fn switched(&self, driver: Arc<dyn ble_gatt::Driver>) -> Arc<dyn ble_gatt::Driver> {
        driver
    }

    /// The driver as the adapter gets it. In a `devtools` build the driver is
    /// behind the test switch (`set_switch`).
    #[cfg(feature = "devtools")]
    fn switched(&self, driver: Arc<dyn ble_gatt::Driver>) -> Arc<dyn ble_gatt::Driver> {
        let switch = test_switch::RadioSwitch::new(driver);
        let _ = self.switch.set(switch.clone());
        Arc::new(test_switch::SwitchedDriver(switch))
    }

    /// Sets the test switch of the Bluetooth radio. Off makes the radio look
    /// switched off to Fini: scans and connections fail, links close, and
    /// `RadioChanged` tells ble-gatt. On makes everything pass through
    /// again. The real radio stays on. False if this device has no usable
    /// adapter.
    #[cfg(feature = "devtools")]
    pub async fn set_switch(&self, on: bool) -> bool {
        if self.adapter().await.is_err() {
            return false;
        }
        match self.switch.get() {
            Some(switch) => {
                switch.set(on).await;
                true
            }
            None => false,
        }
    }

    pub(super) fn note_reachable(&self) {
        self.health.store(ADAPTER_REACHABLE, Ordering::Relaxed);
    }

    pub(super) fn note_unreachable(&self) {
        self.health.store(ADAPTER_UNREACHABLE, Ordering::Relaxed);
    }

    /// Records what a refused scan says about the adapter. A busy adapter (a
    /// discovery already running, which ble-gatt has tried to recover) is
    /// there and working -- only something else refuses it as missing.
    pub(super) fn note_scan_refused(&self, err: &ble_gatt::BleError) -> bool {
        if matches!(err, ble_gatt::BleError::AdapterBusy(_)) {
            self.note_reachable();
            true
        } else {
            self.note_unreachable();
            false
        }
    }

    /// `true` only once a genuine attempt has failed -- never merely because
    /// nothing has been tried yet. See `ADAPTER_UNKNOWN`.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn is_unavailable(&self) -> bool {
        #[cfg(test)]
        if let Some(reachable) = ADAPTER_PINNED.with(|pinned| pinned.get()) {
            return !reachable;
        }
        self.health.load(Ordering::Relaxed) == ADAPTER_UNREACHABLE
    }

    /// Asks the radio, right now, whether it can be used -- and records the
    /// answer into the health like any other attempt.
    ///
    /// The observed signal is free but unhurried: it only refreshes when
    /// the dial loop next scans, which since ADR-0007 is up to 30s away. That
    /// is fine for a row correcting itself in the background and wrong for
    /// the one moment the user is watching -- switching a channel on and
    /// expecting to be told immediately if their own Bluetooth is off. So
    /// this exists to be called from that user action, and only from it.
    ///
    /// It opens a discovery session and drops it immediately rather than
    /// asking BlueZ whether the adapter reports itself powered. An adapter
    /// that claims to be powered and then refuses to scan is a real failure
    /// mode, and the question worth answering is "can we do the thing", not
    /// "does the hardware feel well".
    ///
    /// A scan already running elsewhere answers the question without a
    /// second one: the adapter accepted that discovery session, so it is
    /// reachable. Starting our own would only be refused (see `scan_lock`).
    /// A caller that merely holds the lease proves nothing yet -- its scan
    /// may still be refused -- so the probe waits for it to start or finish.
    #[cfg(any(feature = "ui-plane", test))]
    pub async fn probe(&self, scanner: &Scanner) -> bool {
        let deadline = tokio::time::Instant::now() + PROBE_WAIT_FOR_OTHER_SCAN;
        loop {
            if scanner.scan_is_running() {
                self.note_reachable();
                return true;
            }
            if let Some(_scan) = scanner.try_scan_lock() {
                let Ok(adapter) = self.adapter().await else {
                    self.note_unreachable();
                    return false;
                };
                return match adapter.central().scan(datagram_config().service).await {
                    Ok(stream) => {
                        drop(stream);
                        self.note_reachable();
                        true
                    }
                    Err(err) => {
                        log::warn!("[transport][ble] adapter probe failed: {err}");
                        self.note_scan_refused(&err)
                    }
                };
            }
            // Another caller holds the lease but its scan has not started:
            // wait for it to start (the adapter works) or give the lease up.
            if tokio::time::Instant::now() >= deadline {
                return !self.is_unavailable();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// For a test on this thread that is not about the adapter: the
    /// process-wide health is written by concurrent tests (and by any scan
    /// they start), so a check that reads it would pass or fail with their
    /// timing.
    #[cfg(test)]
    pub fn pin_reachable_on_this_thread() {
        ADAPTER_PINNED.with(|pinned| pinned.set(Some(true)));
    }

    /// For a test about what an unreachable adapter does, for the same
    /// reason.
    #[cfg(test)]
    fn pin_unreachable_on_this_thread() {
        ADAPTER_PINNED.with(|pinned| pinned.set(Some(false)));
    }
}

/// The `actors-ble` e2e lane's radio: a `MockDriver` talking to a
/// cross-process broker instead of BlueZ, so two real `fini-app` processes
/// can prove this file's dial/peripheral/session-claim code path without
/// hardware. See `specs/e2e/actors/helpers/ble-sync.ts` and ble-gatt's
/// `docs/adr/0004-mock-broker-for-cross-process-e2e.md`.
///
/// Both the `devtools` feature gate above (off in release builds) and this
/// env var must be true for a process to ever reach this branch -- an env
/// var alone can never redirect a shipped binary onto a socket.
#[cfg(all(target_os = "linux", feature = "devtools"))]
async fn mock_driver(endpoint: &str) -> Result<Arc<dyn ble_gatt::Driver>, String> {
    use ble_gatt::drivers::mock::{MockDriver, MockNetwork};
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
    Ok(Arc::new(MockDriver::new(
        PeerAddress(address),
        network,
        CapabilityReport { central: true, peripheral: true },
    )) as Arc<dyn ble_gatt::Driver>)
}

/// A test switch over the Bluetooth driver, for debug builds only. It
/// makes the radio look off without a change to the real radio, so that
/// no test must switch off the Bluetooth of a person's device.
#[cfg(feature = "devtools")]
mod test_switch {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ble_gatt::{
        BleError, BoxStream, Driver, CapabilityReport, CharacteristicUuid, ConnectionPriority,
        DiscoveredPeer, GattConnection, GattEvent, GattServiceSpec, PeerAddress, RadioStatus, Role,
        ServiceUuid, WriteType,
    };
    use futures_util::StreamExt;
    use tokio::sync::broadcast;

    fn switched_off() -> BleError {
        BleError::AdapterUnavailable("the test switch set the radio to off".into())
    }

    pub(super) struct RadioSwitch {
        driver: Arc<dyn Driver>,
        off: AtomicBool,
        /// The events the switch adds: `RadioChanged`, and `Disconnected`
        /// for each link that it closes.
        events: broadcast::Sender<GattEvent>,
        /// The open links, from the driver's own events.
        links: Mutex<Vec<(PeerAddress, Role, Option<u64>)>>,
    }

    impl RadioSwitch {
        pub(super) fn new(driver: Arc<dyn Driver>) -> Arc<Self> {
            let switch = Arc::new(Self {
                driver: driver.clone(),
                off: AtomicBool::new(false),
                events: broadcast::channel(64).0,
                links: Mutex::new(Vec::new()),
            });
            let weak = Arc::downgrade(&switch);
            let mut events = driver.events();
            tokio::spawn(async move {
                while let Some(event) = events.next().await {
                    let Some(switch) = weak.upgrade() else { return };
                    switch.record(&event);
                }
            });
            switch
        }

        fn is_off(&self) -> bool {
            self.off.load(Ordering::SeqCst)
        }

        fn record(&self, event: &GattEvent) {
            let mut links = self.links.lock().unwrap();
            match event {
                GattEvent::Connected { peer, local_role, session } => {
                    links.push((peer.clone(), *local_role, *session));
                }
                GattEvent::Disconnected { peer, local_role, session } => {
                    links.retain(|(p, r, s)| !(p.0 == peer.0 && r == local_role && s == session));
                }
                _ => {}
            }
        }

        pub(super) async fn set(&self, on: bool) {
            if on {
                if self.off.swap(false, Ordering::SeqCst) {
                    log::info!("[transport][ble] test switch: radio on");
                    let _ = self.events.send(GattEvent::RadioChanged { status: RadioStatus::On });
                }
                return;
            }
            if self.off.swap(true, Ordering::SeqCst) {
                return;
            }
            log::info!("[transport][ble] test switch: radio off");
            let _ = self.driver.stop_advertising().await;
            let links: Vec<_> = std::mem::take(&mut *self.links.lock().unwrap());
            for (peer, local_role, session) in links {
                if local_role == Role::Peripheral {
                    let _ = self.driver.disconnect_peer(&peer, session).await;
                }
                let _ = self.events.send(GattEvent::Disconnected { peer, local_role, session });
            }
            let _ = self.events.send(GattEvent::RadioChanged { status: RadioStatus::Off });
        }
    }

    pub(super) struct SwitchedDriver(pub(super) Arc<RadioSwitch>);

    #[async_trait]
    impl Driver for SwitchedDriver {
        async fn capabilities(&self) -> CapabilityReport {
            self.0.driver.capabilities().await
        }

        async fn scan(&self, service: ServiceUuid) -> ble_gatt::Result<BoxStream<ble_gatt::Result<DiscoveredPeer>>> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            let switch = self.0.clone();
            let stream = self.0.driver.scan(service).await?;
            Ok(Box::pin(stream.map(move |item| if switch.is_off() { Err(switched_off()) } else { item })))
        }

        async fn connect(&self, peer: &PeerAddress) -> ble_gatt::Result<Box<dyn GattConnection>> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            let inner = self.0.driver.connect(peer).await?;
            Ok(Box::new(SwitchedConnection { inner, switch: self.0.clone() }))
        }

        async fn advertise(&self, service: GattServiceSpec) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.advertise(service).await
        }

        async fn update_advertisement(&self, service: GattServiceSpec) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.update_advertisement(service).await
        }

        async fn stop_advertising(&self) -> ble_gatt::Result<()> {
            self.0.driver.stop_advertising().await
        }

        async fn notify(&self, characteristic: CharacteristicUuid, value: Vec<u8>) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.notify(characteristic, value).await
        }

        async fn notify_peer(
            &self, peer: &PeerAddress, session: Option<u64>, characteristic: CharacteristicUuid, value: Vec<u8>,
        ) -> ble_gatt::Result<()> {
            if self.0.is_off() {
                return Err(switched_off());
            }
            self.0.driver.notify_peer(peer, session, characteristic, value).await
        }

        async fn disconnect_peer(&self, peer: &PeerAddress, session: Option<u64>) -> ble_gatt::Result<()> {
            self.0.driver.disconnect_peer(peer, session).await
        }

        async fn notify_max_len(&self, peer: &PeerAddress, session: Option<u64>) -> Option<usize> {
            self.0.driver.notify_max_len(peer, session).await
        }

        fn events(&self) -> BoxStream<GattEvent> {
            // While the switch is off, writes from centrals do not arrive,
            // as with a radio that is off.
            let switch = self.0.clone();
            let from_driver = self.0.driver.events().filter(move |event| {
                let drop_it = switch.is_off() && matches!(event, GattEvent::CharacteristicWritten { .. });
                std::future::ready(!drop_it)
            });
            let from_switch = futures_util::stream::unfold(self.0.events.subscribe(), |mut rx| async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => return Some((event, rx)),
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            });
            Box::pin(futures_util::stream::select(from_driver, from_switch))
        }

        async fn radio_status(&self) -> RadioStatus {
            if self.0.is_off() {
                return RadioStatus::Off;
            }
            self.0.driver.radio_status().await
        }
    }

    /// A central-role connection that fails while the switch is off.
    struct SwitchedConnection {
        inner: Box<dyn GattConnection>,
        switch: Arc<RadioSwitch>,
    }

    #[async_trait]
    impl GattConnection for SwitchedConnection {
        fn peer(&self) -> PeerAddress {
            self.inner.peer()
        }

        fn session(&self) -> Option<u64> {
            self.inner.session()
        }

        fn att_mtu(&self) -> u16 {
            self.inner.att_mtu()
        }

        async fn read(&mut self, characteristic: CharacteristicUuid) -> ble_gatt::Result<Vec<u8>> {
            if self.switch.is_off() {
                return Err(switched_off());
            }
            self.inner.read(characteristic).await
        }

        async fn write_with_type(
            &mut self, characteristic: CharacteristicUuid, value: Vec<u8>, write_type: WriteType,
        ) -> ble_gatt::Result<()> {
            if self.switch.is_off() {
                return Err(switched_off());
            }
            self.inner.write_with_type(characteristic, value, write_type).await
        }

        async fn subscribe(
            &mut self, characteristic: CharacteristicUuid,
        ) -> ble_gatt::Result<BoxStream<ble_gatt::Result<Vec<u8>>>> {
            if self.switch.is_off() {
                return Err(switched_off());
            }
            let switch = self.switch.clone();
            let stream = self.inner.subscribe(characteristic).await?;
            Ok(Box::pin(stream.take_while(move |_| std::future::ready(!switch.is_off()))))
        }

        async fn disconnect(&mut self) -> ble_gatt::Result<()> {
            self.inner.disconnect().await
        }

        async fn request_connection_priority(&mut self, priority: ConnectionPriority) -> ble_gatt::Result<()> {
            self.inner.request_connection_priority(priority).await
        }
    }
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
/// Only a real *use* records health. Recording success from `adapter()`
/// alone would be wrong: it caches the adapter in a `OnceCell`, so once the
/// adapter has worked once, every later call returns that cached handle
/// without touching hardware -- an adapter switched off afterwards would
/// still look reachable, and the row would flap between the cached success
/// and the scan failure that follows it.
const ADAPTER_UNKNOWN: u8 = 0;
const ADAPTER_REACHABLE: u8 = 1;
const ADAPTER_UNREACHABLE: u8 = 2;

#[cfg(test)]
thread_local! {
    // The adapter health a test pinned on its thread, if any.
    static ADAPTER_PINNED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
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
    static MOCK_DRIVER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Proves `mock_driver` actually round-trips through a real socket to a
    /// real `MockNetwork::serve` broker, not just that it compiles -- the
    /// same connect+advertise+scan shape `ble-gatt`'s own
    /// `tests/mock_broker.rs` exercises, but through fini's injection point
    /// (`FINI_BLE_MOCK_BROKER`/`FINI_LOCAL_BLUETOOTH_ADDRESS`) instead of
    /// calling `MockNetwork::remote` directly. This is what the `actors-ble`
    /// e2e lane's two real `fini-app` processes will each do at startup.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    #[tokio::test]
    async fn mock_driver_connects_to_a_broker_and_reports_full_capabilities() {
        let _guard = MOCK_DRIVER_ENV_LOCK.lock().unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback broker port");
        let broker_addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = ble_gatt::drivers::mock::MockNetwork::serve(listener).await;
        });

        std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "AA:BB:CC:00:00:01");
        let result = mock_driver(&broker_addr.to_string()).await;
        std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");

        let driver = result.expect("mock_driver should connect to the broker");
        let capabilities = driver.capabilities().await;
        assert!(capabilities.central && capabilities.peripheral);
    }

    /// The one precondition `mock_driver` itself enforces: without a local
    /// address there is nothing to identify this peer as on the mock radio,
    /// and the error must name which env var is missing rather than panic
    /// or silently pick something.
    #[cfg(all(target_os = "linux", feature = "devtools"))]
    #[tokio::test]
    async fn mock_driver_requires_a_local_bluetooth_address() {
        let _guard = MOCK_DRIVER_ENV_LOCK.lock().unwrap();
        std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");

        let err = match mock_driver("127.0.0.1:1").await {
            Ok(_) => panic!("must fail without a local address"),
            Err(err) => err,
        };
        assert!(err.contains("FINI_LOCAL_BLUETOOTH_ADDRESS"));
    }

    /// ble-gatt reports a discovery it could not take over as busy, not
    /// missing: that adapter works, so Bluetooth must not read as unavailable.
    #[test]
    fn a_busy_adapter_is_not_an_unavailable_one() {
        let adapter = BluetoothAdapter::this_device();
        assert!(adapter.note_scan_refused(&ble_gatt::BleError::AdapterBusy("in progress".into())));
        assert!(!adapter.note_scan_refused(&ble_gatt::BleError::AdapterUnavailable("no adapter".into())));
    }

    /// A scan already running is proof the adapter works, so the check a
    /// person triggers by switching Bluetooth on must say so -- not start a
    /// second scan, which Android refuses ("a scan is already active") and
    /// which used to be recorded as "Bluetooth is unavailable on this device".
    #[tokio::test]
    async fn adapter_probe_during_another_scan_reports_the_adapter_reachable() {
        let radio = Scanner::default();
        let _other_scan = radio.scan_lock().await;
        let _running = radio.start_running_scan();
        let adapter = BluetoothAdapter::this_device();
        adapter.note_unreachable();

        assert!(
            adapter.probe(&radio).await,
            "a scan in flight means the adapter accepted a discovery session"
        );
        assert!(!adapter.is_unavailable());
    }

    /// Holding the lease is not a running scan. The add-mode scan takes the
    /// lease and then waits on the adapter; a probe in that window used to
    /// report a missing radio as working, so adding Bluetooth never failed.
    #[tokio::test(start_paused = true)]
    async fn adapter_probe_does_not_trust_a_scan_that_has_not_started() {
        let radio = Arc::new(Scanner::default());
        let other_scan = radio.scan_lock().await;
        // Pinned, not recorded: other tests' scans write the process-wide
        // health while this one waits out the probe.
        BluetoothAdapter::pin_unreachable_on_this_thread();

        let probe = tokio::spawn({
            let radio = radio.clone();
            async move { BluetoothAdapter::this_device().probe(&radio).await }
        });
        tokio::time::sleep(PROBE_WAIT_FOR_OTHER_SCAN + Duration::from_secs(1)).await;
        drop(other_scan);

        assert!(!probe.await.unwrap(), "no scan ever started, and the adapter was last seen unreachable");
    }
}
