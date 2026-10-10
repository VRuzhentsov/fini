//! Finding a device to pair with, and saying hello to a paired one:
//! the add-mode candidate scan (ADR 0002 Phase 3) and the setup search
//! (ADR-0008 D1, D12).

use super::*;

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
///
/// Then from 12s to 25s, because 12s was still under the *backend's* own
/// bound and so could never express anything but "give up early". ble-gatt
/// gives a connect `CONNECT_TIMEOUT` (20s) to finish; a probe capped below
/// that is guaranteed to abandon a dial the backend had not yet given up on.
/// Measured here, Add Device found the peer every pass and never once
/// probed it successfully:
///
///   connect: dialling 6E:1C:BD:59:00:20            12:25:35
///   connect: abandoned before completing           12:25:47   <- 12.0s
///     (connect guard dropped); quarantining and cleaning up in the background
///
/// and the quarantine then also broke the *next* pass, so the picker stayed
/// empty for as long as anyone cared to watch. A GATT connect is simply this
/// slow: 10.0-10.5s phone-to-desktop on every one of four passes, and
/// `SESSION_CONNECT_TIMEOUT` below records ~28s seen elsewhere. So the cap
/// has to sit above the backend's own, and its job is only to stop a silent
/// peer from eating the pass -- never to pre-empt a dial still in progress.
///
/// *Just* above `CONNECT_TIMEOUT` rather than comfortably above it, because
/// `dial_for_pairing` waits out a probe already in flight, and
/// `SEND_PAIR_BLE_TIMEOUT` has to cover that wait *plus* its own dial within
/// `PAIR_REQUEST_TTL_SECS`. A probe still running past the backend's own
/// bound is one the backend has already given up on, so further margin buys
/// nothing here and comes straight out of the pairing budget. Keep the two
/// in step: raising this lengthens the worst case Pair has to sit through.
#[cfg(any(feature = "ui-plane", test))]
const CANDIDATE_PROBE_TIMEOUT: Duration = Duration::from_secs(25);

/// Once the listening phase of a candidate scan has heard its first
/// advertiser, it keeps listening this much longer (for a second device,
/// say) and then moves on to probing. The scan window is long (a minute,
/// `BLUETOOTH_SCAN_DURATION_MS`) so a slow adapter gets time, but a
/// candidate is only reported when the whole call returns: without this the
/// picker would sit empty for the full listening half after the peer was
/// already heard.
#[cfg(any(feature = "ui-plane", test))]
const ADVERTISER_SETTLE: Duration = Duration::from_secs(3);

/// How long the device with the higher id waits, in a Bluetooth setup, for the
/// other device's hello to reach it before dialling anyway. See
/// `setup_hello_round`.
#[cfg(all(feature = "ui-plane", not(test)))]
const HIGHER_ID_PATIENCE: Duration = Duration::from_secs(15);
#[cfg(test)]
const HIGHER_ID_PATIENCE: Duration = Duration::from_millis(1_500);

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
    let mut link = dial_session(state, peer_id, address).await.ok()?;
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
    let radio = state.bluetooth_radio.clone();
    let Some(found) = radio.find(&peer_id, Purpose::Setup, timeout).await else {
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
    let _probe = radio.candidate_probe().await;
    let acknowledged = tokio::time::timeout(
        FIND_PEER_CANDIDATE_TIMEOUT,
        hello_candidate(&state, &found.address, &peer_id),
    )
    .await
    .ok()
    .flatten()
    .is_some();
    if !acknowledged {
        radio.note_dial_failed(&found.address);
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
#[derive(Clone)]
pub struct AddModeCandidate {
    pub address: String,
    pub device_id: String,
    pub hostname: String,
}

/// Dials `address` and exchanges `DiscoveryHello`/`DiscoveryHelloReply`.
/// `None` on any failure along the way (dial, send, no/wrong reply); the
/// caller is responsible for bounding how long this is allowed to run.
#[cfg(any(feature = "ui-plane", test))]
async fn probe_discovery_hello(radio: &RadioArbiter, hello: &PeerFrame, address: &str) -> Option<PeerFrame> {
    // The add-mode scan has closed by now, but a search for a paired peer
    // may start one; registering the dial keeps it paused until we finish.
    let _probe = radio.candidate_probe().await;
    let _dial = radio.acquire_dial().await;
    let mut link = dial(address).await.ok()?;
    send_frame(link.as_mut(), hello).await.ok()?;
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
    my_device_id: &str, timeout: Duration, peers: &PeerDirectory, radio: &RadioArbiter, hello: &PeerFrame,
) -> Result<Vec<AddModeCandidate>, String> {
    let mut legs = radio.watch_pairing_legs();
    // Wait out a pairing leg already running, then run the pass and give it
    // up when one starts. An `Err` keeps the picker's previous list (the
    // caller treats it as "retry later"), where an empty `Ok` would wipe the
    // very candidate being paired.
    let _ = legs.wait_for(|count| *count == 0).await;
    let backend = backend().await.inspect_err(|_| note_adapter_unreachable())?;
    scan_add_mode_candidates_pass(my_device_id, timeout, &mut legs, backend, peers, radio, |address| async move {
        probe_discovery_hello(radio, hello, &address).await
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
    peers: &PeerDirectory,
    radio: &RadioArbiter,
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
    // Advertisers that dial this device rather than being dialled by it,
    // with their fingerprints; see below.
    let mut answered_by: Vec<(String, [u8; FINGERPRINT_LEN])> = Vec::new();
    let flagged_addresses = {
        let _scan = radio.scan_lease_between_dials().await;
        let mut discovered = backend
            .scan(datagram_config().service)
            .await
            .inspect_err(|err| {
                note_scan_refused(err);
            })
            .map_err(|err| format!("ble scan failed: {err}"))?;
        let _running = radio.start_running_scan();

        // Listening gets at most half the caller's window, so the probe
        // phase always has something left. Splitting scan from probe fixed
        // one bug and introduced the risk of another: a scan that runs to
        // the full deadline leaves zero budget for the dials it just queued
        // up, and the pass returns nothing having done nothing -- looking
        // exactly like "no candidates" while actually meaning "no time".
        let mut listen_deadline = deadline - timeout / 2;

        let mut flagged: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mine = fingerprint_of(my_device_id);
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
            //
            // Only one of two devices dials the other: the lower
            // fingerprint. Both dialling each other crossed connections to
            // the same device, and BlueZ tore both down. The higher side
            // lists the lower one from the hello it receives instead
            // (`inbound_hellos`), at the address it advertises from.
            let theirs = advertised_fingerprint(
                peer.manufacturer_data.get(&FINI_MANUFACTURER_ID).map(Vec::as_slice),
            );
            if let Some(theirs) = theirs {
                peers.note_advertiser(&address, theirs);
            }
            match theirs {
                Some(theirs) if theirs <= mine => answered_by.push((address, theirs)),
                _ => flagged.push(address),
            }
        }
        // Logged unconditionally, at info. Three separate hypotheses about
        // why add-mode discovery finds nothing have now been wrong, each
        // costing a build/deploy/hardware cycle, because the only evidence
        // available was `scan: discovered` lines from ble-gatt that cannot
        // distinguish this scan from the dial loop's. This line says what
        // *this* call saw and what it will probe, which is the fact every
        // one of those attempts was missing.
        log::info!(
            "[transport][ble] add-mode scan saw {} advertiser(s), probing {}, leaving {} to dial us",
            seen.len(),
            flagged.len(),
            answered_by.len()
        );
        flagged
        // `discovered` is dropped here, stopping discovery, before any
        // probe below runs.
    };

    let mut candidates = Vec::new();
    for address in flagged_addresses {
        // The scan deadline decides which candidates are *started*, and
        // `CANDIDATE_PROBE_TIMEOUT` bounds each one that is: one
        // unresponsive candidate (in range, advertising, but slow or gone
        // by the time this connects) must not eat the whole scan -- the
        // frontend runs this as a single self-rescheduling chain, so one
        // stuck candidate here would otherwise delay every subsequent Add
        // Device discovery pass. Dial and send are covered too, not just
        // the reply: neither has a bound of its own.
        //
        // The deadline deliberately does *not* also clip a probe already
        // running. A probe in flight is never cancelled -- not for pairing,
        // and not for the caller's window either: abandoning a dial makes
        // ble-gatt quarantine the address and disconnect it in the
        // background, which removes the device from BlueZ and fails both
        // the pairing dial that follows and the next pass's probe. So the
        // pass may overrun `duration_ms` by its last candidate, and stops
        // before starting another.
        if *legs.borrow() > 0 {
            return Err(PAIRING_PAUSED.to_string());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        // `CANDIDATE_PROBE_TIMEOUT` outright, not clipped to `remaining`:
        // the deadline decides whether another probe *starts* (the check
        // just above), and once one has, cutting it short is the one
        // outcome this loop must not produce. A dial abandoned part-way is
        // not a probe that merely failed -- ble-gatt quarantines the
        // address and disconnects it in the background, so the pass after
        // this one cannot reach the peer either. Overrunning the caller's
        // window by one slow candidate is the cheaper of the two.
        if let Some(candidate) = peers.recent_probe(&address) {
            candidates.push(candidate);
            continue;
        }
        let reply = tokio::time::timeout(CANDIDATE_PROBE_TIMEOUT, probe(address.clone())).await;
        if let Ok(Some(PeerFrame::DiscoveryHelloReply { device_id, hostname, endpoint_id })) = reply {
            // A stale/self-seen advertisement (e.g. two adapters on the
            // same machine, or a previous scan's own peripheral still
            // winding down) must not show up as a candidate to pair with.
            if device_id != my_device_id {
                let candidate = AddModeCandidate { address, device_id, hostname };
                peers.note_probe(&candidate, endpoint_id);
                candidates.push(candidate);
            }
        }
    }
    // The advertisers left to dial this device: listed from the hello they
    // sent, at the address they advertise from -- the one a pairing leg
    // from this side can reach.
    for (address, fingerprint) in answered_by {
        if let Some(candidate) = peers.prober_at(&address, fingerprint, my_device_id) {
            if !candidates.iter().any(|c| c.device_id == candidate.device_id) {
                candidates.push(candidate);
            }
        }
    }
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            endpoint_id: format!("key-{address}"),
        }
    }

    /// A pass that finds a pairing step already running gives up with the
    /// paused error -- which keeps the picker's list -- and dials nobody.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_candidate_scan_stands_aside_while_a_pairing_step_runs() {
        let radio = RadioArbiter::default();
        let (scanner, advertisers) = mock_radio_with_advertisers(&["AA:00:00:00:01:01"]);
        advertise_all(&advertisers).await;
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let leg = radio.begin_pairing_leg();
        let mut legs = radio.watch_pairing_legs();
        let peers = PeerDirectory::default();
        let result = scan_add_mode_candidates_pass("me", Duration::from_secs(10), &mut legs, scanner, &peers, &radio, |address| {
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
        let radio = Arc::new(RadioArbiter::default());
        let leg = radio.begin_pairing_leg();
        let scan = tokio::spawn({
            let radio = radio.clone();
            async move {
                let hello = PeerFrame::Hello { device_id: "me".to_string() };
                scan_add_mode_candidates("me", Duration::from_millis(50), &PeerDirectory::default(), &radio, &hello)
                    .await
            }
        });
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
        let radio = Arc::new(RadioArbiter::default());
        let (scanner, advertisers) = mock_radio_with_advertisers(&["AA:00:00:00:02:01", "AA:00:00:00:02:02"]);
        advertise_all(&advertisers).await;
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let finished = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());

        let pass = {
            let (started, finished, release) = (started.clone(), finished.clone(), release.clone());
            let radio = radio.clone();
            tokio::spawn(async move {
                let mut legs = radio.watch_pairing_legs();
                let peers = PeerDirectory::default();
                scan_add_mode_candidates_pass("me", Duration::from_secs(10), &mut legs, scanner, &peers, &radio, move |address| {
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

        let leg = radio.begin_pairing_leg();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!pass.is_finished(), "the probe in flight is not cancelled");
        release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(2), pass).await.unwrap().unwrap();
        drop(leg);

        assert_eq!(finished.load(Ordering::SeqCst), 1, "the probe in flight ran to its end");
        assert_eq!(started.load(Ordering::SeqCst), 1, "no probe starts after the pairing step began");
        assert_eq!(result.err().as_deref(), Some(PAIRING_PAUSED));
    }
}
