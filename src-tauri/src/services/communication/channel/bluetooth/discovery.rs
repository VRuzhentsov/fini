//! Identify a device to pair with, and say hello to a paired one: the
//! identify step that Pair starts (option B: the list needs no connection),
//! and the setup search (ADR-0008 D1, D12).

use super::*;

/// How long the identify step waits for the answer once the link is open.
/// The connection itself is never cut short: ble-gatt gives it its own 20 s,
/// and an abandoned connect makes BlueZ crash (two crashes in the spike).
#[cfg(any(feature = "ui-plane", test))]
const IDENTIFY_ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the device with the higher id waits, in a Bluetooth setup, for the
/// other device's hello to reach it before dialling anyway. See
/// `setup_hello_round`.
#[cfg(all(feature = "ui-plane", not(test)))]
const HIGHER_ID_PATIENCE: Duration = Duration::from_secs(3);
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
/// Fini install; the scan itself already filters to Fini's service
/// UUID (each platform driver does this at the native scan-callback level, before
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
    let radio = state.bluetooth_scanner.clone();
    let Some(found) = radio.find(&peer_id, Purpose::Setup, timeout).await else {
        if BluetoothAdapter::this_device().is_unavailable() {
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
pub struct AvailableDevice {
    pub address: String,
    pub device_id: String,
    pub hostname: String,
}

/// Dials `address` and exchanges `DiscoveryHello`/`DiscoveryHelloReply`.
/// `None` on any failure along the way (dial, send, no/wrong reply); the
/// caller is responsible for bounding how long this is allowed to run.
#[cfg(any(feature = "ui-plane", test))]
async fn identify_device(radio: &Scanner, hello: &PeerFrame, address: &str) -> Option<PeerFrame> {
    // A search for a paired device may start a scan; holding a connect
    // guard keeps it paused until this step ends.
    let _probe = radio.candidate_probe().await;
    let _connect = radio.acquire_connect().await;
    let mut link = dial(address).await.ok()?;
    send_frame(link.as_mut(), hello).await.ok()?;
    tokio::time::timeout(IDENTIFY_ANSWER_TIMEOUT, recv_frame(link.as_mut())).await.ok()??.ok()
}

/// Pair pressed on a device the Add Device list shows from its
/// advertisement: one connection to the newest address the scan heard it at,
/// to learn its device id, name and iroh key. The result is recorded, so the
/// pair request that follows can connect over iroh.
#[cfg(any(feature = "ui-plane", test))]
pub async fn identify_heard_device(
    state: &DeviceConnectionState, fingerprint: [u8; FINGERPRINT_LEN],
) -> Result<AvailableDevice, String> {
    let scanner = &state.bluetooth_scanner;
    let address = scanner
        .available
        .heard_address(fingerprint)
        .ok_or_else(|| "that device is not heard any more -- open Add Device on it again".to_string())?;
    let started = std::time::Instant::now();
    let reply = identify_device(scanner, state.bluetooth_advertiser.hello(), &address).await;
    log::info!(
        "[transport][ble] identify {address} took {:?}: {}",
        started.elapsed(),
        if reply.is_some() { "answered" } else { "no answer" }
    );
    match reply {
        Some(PeerFrame::DiscoveryHelloReply { device_id, hostname, endpoint_id }) => {
            if device_id == state.identity.device_id {
                return Err("that is this device".to_string());
            }
            let device = AvailableDevice { address, device_id, hostname };
            scanner.available.record_identified(&device, endpoint_id);
            Ok(device)
        }
        _ => Err("the device did not answer -- make sure Add Device is open on it".to_string()),
    }
}
