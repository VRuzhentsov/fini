//! End-to-end proof that the transport abstraction works: two independent
//! adapters (`tcp_ws`, `sim`) carry the exact same `session::run_peer_gate`/
//! `run_session` engine, both transports can be simultaneously connected
//! for the same peer (ADR-0003 revision), and both satisfy the `Transport`
//! trait polymorphically. This is the protocol-level coverage referenced by
//! the E2E topology matrix in `specs/e2e/transports.md` — it proves
//! per-transport claiming/primary-selection semantics without needing a
//! real Android runtime or a real radio.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use diesel::prelude::*;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;

use crate::models::CreatePairedDeviceInput;
use crate::schema::paired_devices;
use crate::services::db::{open_db_at_path, temp_db_path};
use crate::services::device_connection::DeviceConnectionState;
use crate::services::space_sync::session;
use crate::services::space_sync::types::PeerFrame;
use crate::services::transport::{recv_frame, send_frame, sim, tcp_ws, Link, Transport, TransportKind};

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn seed_paired_device(db_path: &PathBuf, peer_device_id: &str) {
    let mut conn = open_db_at_path(db_path);
    diesel::insert_into(paired_devices::table)
        .values(&CreatePairedDeviceInput {
            peer_device_id: peer_device_id.to_string(),
            display_name: "Test Peer".to_string(),
            paired_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .execute(&mut conn)
        .expect("seed paired device");
}

/// Marks `peer_device_id` Bluetooth-enabled with a stored address --
/// `device_connection_set_preferred_transport_impl`'s own eligibility
/// check (`peer_is_currently_bluetooth_eligible`) also requires a live OS
/// bond, which callers must separately arrange via the
/// `FINI_BLUETOOTH_PAIRED_ADDRESSES` escape hatch (holding
/// `BLUETOOTH_ADDRESS_ENV_LOCK`) for the address used here.
fn seed_bluetooth_enabled_peer(db_path: &PathBuf, peer_device_id: &str, address: &str) {
    let mut conn = open_db_at_path(db_path);
    diesel::update(paired_devices::table.find(peer_device_id))
        .set((
            paired_devices::bluetooth_enabled.eq(true),
            paired_devices::bluetooth_address.eq(Some(address)),
        ))
        .execute(&mut conn)
        .expect("mark peer bluetooth-enabled with an address");
}

fn server_state(label: &str) -> (DeviceConnectionState, PathBuf) {
    let db_path = temp_db_path(label);
    let data_dir = db_path.with_extension("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // FINI_MDNS_DISABLED keeps these tests hermetic (no real mDNS daemon).
    std::env::set_var("FINI_MDNS_DISABLED", "1");
    let state = DeviceConnectionState::from_app_data_dir(&data_dir);
    (state, db_path)
}

/// `FINI_SPACE_SYNC_WS_PORT` is process-global and read once at
/// `DeviceConnectionState` construction; serialize the read+construct
/// window so concurrently-running tests can't clobber each other's value.
static WS_PORT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `FINI_BLUETOOTH_PAIRED_ADDRESSES` is process-global too. Shared with
/// `device_connection::commands::tests` (not a separate lock of the same
/// name -- an earlier version of this comment claimed a disjoint set of
/// tests justified two locks, but both modules set/clear the exact same
/// env var, so two locks could still race with *each other*, observed as
/// intermittent failures once enough tests in both files touched it).
use crate::services::device_connection::BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK as BLUETOOTH_ADDRESS_ENV_LOCK;

/// Like `server_state`, but the constructed state *announces* `port` as its
/// own `space_sync_ws_port` (what it puts in outgoing `PairRequestPayload.from_ws_port`
/// for peers to reply to) — needed whenever a test's peer must reply back to
/// this actor's listener rather than just being dialed by it. Never rely on
/// the crate's hardcoded default port (`45455`) in a test: a real, unrelated
/// app instance may already be listening on it on the host running the test.
fn server_state_on_port(label: &str, port: u16) -> (DeviceConnectionState, PathBuf) {
    let _guard = WS_PORT_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_SPACE_SYNC_WS_PORT", port.to_string());
    let result = server_state(label);
    std::env::remove_var("FINI_SPACE_SYNC_WS_PORT");
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_ws_gate_accepts_paired_device_and_claims_session_as_network() {
    let (server, server_db) = server_state("transport-tcpws-accept");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");

    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::TcpWs)
    );
}

/// ADR-0003 revision: pinning a peer to a transport different from the one
/// currently primary just flips which already-connected transport is
/// primary -- no wire frame, no session disturbed on either transport,
/// since both stay connected regardless of the pin.
#[tokio::test(flavor = "multi_thread")]
async fn set_preferred_transport_flips_primary_without_disturbing_either_session() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-set-preferred-flips-primary");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    let tcp_port = free_port().await;
    let sim_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), tcp_port));
    tokio::spawn(sim::run_server(server.clone(), server_db.clone(), sim_port));
    sleep(Duration::from_millis(100)).await;

    let mut tcp_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port).await.expect("dial tcp");
    session::perform_client_auth(tcp_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("tcp auth should succeed for paired device");
    let mut sim_link = AsBluetooth(sim::dial(sim_port).await.expect("dial sim"));
    session::perform_client_auth(&mut sim_link, "peer-client", &server.identity.device_id)
        .await
        .expect("sim (bluetooth-kind) auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.primary_transport("peer-client"), Some(TransportKind::TcpWs));

    let mut conn = open_db_at_path(&server_db);
    let updated = crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(TransportKind::Bluetooth),
    )
    .expect("set preferred transport");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    assert_eq!(updated.preferred_transport.as_deref(), Some("bluetooth"));
    assert!(updated.preferred_transport_set_at.is_some());

    // The server-side session actually claims under the real wire kind
    // (`Sim`, standing in for Bluetooth here) -- `AsBluetooth` only affects
    // what the *client* reports, not what `run_peer_gate`'s accept side
    // sees from its own `link.kind()`. `transport_kind_to_preference_string`
    // collapses Sim/Bluetooth/LoRa to the same "bluetooth" pin either way.
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::Sim),
        "primary must flip immediately, without waiting for a reconnect"
    );
    assert!(
        server.has_session_on("peer-client", TransportKind::TcpWs),
        "the Network session must stay connected -- only primary-ness changed"
    );
    assert!(
        server.has_session_on("peer-client", TransportKind::Sim),
        "the bluetooth-role session must stay connected"
    );
}

/// Regression test for a P1 review finding: a stale "configured" row
/// (`DeviceView`'s polling only refreshes session liveness, not full
/// eligibility -- see the frontend's own `refreshLiveConnectedState` doc
/// comment) can stay clickable well after the OS Bluetooth bond quietly
/// disappears. `device_connection_set_preferred_transport_impl` must
/// re-validate current eligibility itself rather than trusting the click --
/// persisting and announcing a pin that can never actually connect just
/// relocates the stranding hazard instead of preventing it.
#[tokio::test(flavor = "multi_thread")]
async fn set_preferred_transport_refuses_a_bluetooth_pin_when_not_currently_eligible() {
    let (server, server_db) = server_state("transport-set-preferred-bluetooth-ineligible");
    seed_paired_device(&server_db, "peer-client");
    // Bluetooth left disabled (the schema default) -- the condition under
    // test; no `FINI_BLUETOOTH_PAIRED_ADDRESSES` escape hatch either, so
    // even a stored address wouldn't pass the OS-bond check.

    let mut conn = open_db_at_path(&server_db);
    let err = crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(TransportKind::Bluetooth),
    )
    .expect_err("must refuse a Bluetooth pin that isn't currently eligible");
    assert!(err.contains("Bluetooth"), "got: {err}");

    let row: crate::models::PairedDevice = paired_devices::table
        .find("peer-client")
        .select(crate::models::PairedDevice::as_select())
        .first(&mut conn)
        .expect("load peer row");
    assert_eq!(row.preferred_transport, None, "a refused pin must not be persisted");
}

/// ADR-0003 revision: both transports now dial/accept and stay connected
/// independent of the manual pin -- the pin only decides which
/// already-connected transport is primary. Proves the network dial loop
/// still establishes a session for a peer explicitly pinned to Bluetooth,
/// with no override/eligibility gating needed on the dial path itself.
#[tokio::test(flavor = "multi_thread")]
async fn network_dial_establishes_regardless_of_a_bluetooth_pin() {
    let (responder, responder_db) = server_state("transport-network-dial-ignores-pin-responder");
    let (dialer, dialer_db) = server_state("transport-network-dial-ignores-pin-dialer");
    seed_paired_device(&responder_db, &dialer.identity.device_id);
    seed_paired_device(&dialer_db, &responder.identity.device_id);

    let mut dialer_conn = open_db_at_path(&dialer_db);
    diesel::update(paired_devices::table.find(&responder.identity.device_id))
        .set(paired_devices::preferred_transport.eq(Some("bluetooth")))
        .execute(&mut dialer_conn)
        .expect("pin the dialer to bluetooth directly (bypassing the impl's own eligibility check)");

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(responder.clone(), responder_db.clone(), port));
    sleep(Duration::from_millis(100)).await;
    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", port);

    tokio::spawn(tcp_ws::dial_with_backoff(
        dialer.clone(),
        dialer_db,
        responder.identity.device_id.clone(),
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut established = false;
    while tokio::time::Instant::now() < deadline {
        if dialer.has_session_on(&responder.identity.device_id, TransportKind::TcpWs) {
            established = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        established,
        "a Bluetooth pin must not suppress the Network dial loop"
    );
}

/// Regression test for a P1 review finding: the in-flight-dial guard added
/// to `spawn_dial_loop` means a peer's retry task is never re-spawned with
/// fresh data while one is already running -- so `dial_with_backoff` must
/// itself notice when the peer's presenced endpoint changes and adopt it,
/// not keep retrying whatever address it happened to see on its first
/// iteration. Seeds presence at a dead port first (so the first attempt
/// fails fast), then updates presence to a real listening server mid-retry
/// and confirms the loop picks up the new endpoint on its own.
#[tokio::test(flavor = "multi_thread")]
async fn dial_with_backoff_adopts_a_changed_endpoint_mid_retry() {
    let (responder, responder_db) = server_state("transport-dial-adopts-changed-endpoint-responder");
    let (dialer, dialer_db) = server_state("transport-dial-adopts-changed-endpoint-dialer");
    seed_paired_device(&responder_db, &dialer.identity.device_id);
    seed_paired_device(&dialer_db, &responder.identity.device_id);

    // A bound-then-dropped port: connecting to it fails fast (connection
    // refused) rather than hanging, so the first retry iteration completes
    // quickly without needing to wait out a real timeout.
    let dead_port = free_port().await;
    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", dead_port);

    tokio::spawn(tcp_ws::dial_with_backoff(
        dialer.clone(),
        dialer_db,
        responder.identity.device_id.clone(),
    ));
    sleep(Duration::from_millis(200)).await;
    assert!(
        !dialer.has_session_on(&responder.identity.device_id, TransportKind::TcpWs),
        "must not have established anything against the dead port"
    );

    let real_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(responder.clone(), responder_db.clone(), real_port));
    sleep(Duration::from_millis(100)).await;
    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", real_port);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut established = false;
    while tokio::time::Instant::now() < deadline {
        if dialer.has_session_on(&responder.identity.device_id, TransportKind::TcpWs) {
            established = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        established,
        "the retry loop must adopt the peer's updated endpoint instead of retrying the stale one forever"
    );
}

/// Regression test for a P1 review finding: disabling Bluetooth for a pair
/// that was pinned to it must not leave this device pinned to a transport
/// that can never reconnect -- `device_connection_set_bluetooth_transport_
/// with_state_impl` must redirect the pin to Network, which immediately
/// becomes primary since it's already connected (no wire notification
/// needed any more: both transports stay connected regardless of the pin,
/// so there's nothing to relay to the peer).
#[tokio::test(flavor = "multi_thread")]
async fn disabling_bluetooth_redirects_the_pin_to_network_and_flips_primary() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-disable-bluetooth-redirects");
    seed_paired_device(&server_db, "peer-client");

    let tcp_port = free_port().await;
    let sim_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), tcp_port));
    tokio::spawn(sim::run_server(server.clone(), server_db.clone(), sim_port));
    sleep(Duration::from_millis(100)).await;

    let mut tcp_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port).await.expect("dial tcp");
    session::perform_client_auth(tcp_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("tcp auth should succeed for paired device");
    let mut sim_link = AsBluetooth(sim::dial(sim_port).await.expect("dial sim"));
    session::perform_client_auth(&mut sim_link, "peer-client", &server.identity.device_id)
        .await
        .expect("sim (bluetooth-kind) auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;

    let mut conn = open_db_at_path(&server_db);
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(TransportKind::Bluetooth),
    )
    .expect("pin to bluetooth");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    // See the sibling test above for why this is `Sim`, not `Bluetooth`.
    assert_eq!(server.primary_transport("peer-client"), Some(TransportKind::Sim));

    let updated = crate::services::device_connection::device_connection_set_bluetooth_transport_with_state_impl(
        &mut conn,
        &server,
        crate::services::device_connection::types::DeviceBluetoothTransportInput {
            peer_device_id: "peer-client".to_string(),
            enabled: false,
            bluetooth_address: None,
        },
    )
    .expect("disable bluetooth");
    assert_eq!(
        updated.preferred_transport.as_deref(),
        Some("network"),
        "disabling a Bluetooth pin must redirect to Network, not merely clear it"
    );
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::TcpWs),
        "primary must flip to the already-connected Network session immediately"
    );
}

/// Regression test for a second P1 review finding on this PR: disabling
/// Bluetooth while a `TransportKind::Bluetooth` session is already live
/// must not let that session win primary-transport fallback later, even in
/// the window before its own async teardown (`close_session_on`'s
/// fire-and-forget `SessionCommand::Close`) has been processed --
/// otherwise `push_to_peer` could resume real application traffic over a
/// transport the user just explicitly turned off, violating
/// `specs/device-connect/README.md`'s "disabling ... prevents future
/// Bluetooth use" contract. Deliberately does *not* sleep between disabling
/// and dropping Network, so this exercises `recompute_primary_locked`'s
/// `bluetooth_enabled` exclusion directly rather than depending on the
/// async close having (or not having) already run. Uses `AsBluetooth`
/// (real Bluetooth-kind claiming, unlike the sibling test above's `Sim`) --
/// `bluetooth_enabled` deliberately only excludes the real `Bluetooth`
/// kind, not `Sim`/`LoRa`, which aren't governed by that column at all
/// (see `recompute_primary_locked`'s own doc comment), so this needs the
/// real kind to exercise the check meaningfully.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_bluetooth_excludes_it_from_primary_fallback_even_before_its_session_closes() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "127.0.0.1");

    let (server, server_db) = server_state("transport-disable-bluetooth-excludes-fallback");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some("127.0.0.1")),
            ))
            .execute(&mut conn)
            .expect("enable bluetooth for seeded peer");
    }

    let tcp_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), tcp_port));

    let ble_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ble_port = ble_listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = ble_listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_server, gate_db).await;
    });
    sleep(Duration::from_millis(100)).await;

    let mut tcp_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port).await.expect("dial tcp");
    session::perform_client_auth(tcp_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("tcp auth should succeed for paired device");

    let ble_stream = TcpStream::connect(("127.0.0.1", ble_port)).await.unwrap();
    let mut ble_link: Box<dyn Link> = Box::new(sim::SimLink::new(ble_stream));
    session::perform_client_auth(ble_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("bluetooth-kind auth should succeed for a bonded, enabled paired device");
    sleep(Duration::from_millis(50)).await;

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    // Network is primary by default (network-first) -- the common
    // real-world case for this finding: disabling Bluetooth from Settings
    // while a healthy Network session is already carrying traffic, not the
    // pinned case the sibling test above covers.
    assert_eq!(server.primary_transport("peer-client"), Some(TransportKind::TcpWs));

    let mut conn = open_db_at_path(&server_db);
    crate::services::device_connection::device_connection_set_bluetooth_transport_with_state_impl(
        &mut conn,
        &server,
        crate::services::device_connection::types::DeviceBluetoothTransportInput {
            peer_device_id: "peer-client".to_string(),
            enabled: false,
            bluetooth_address: None,
        },
    )
    .expect("disable bluetooth");

    // Network drops immediately after -- no sleep, so the Bluetooth
    // session's own `run_session` loop has not necessarily processed the
    // `Close` command `close_session_on` just sent it. Without
    // `recompute_primary_locked`'s `bluetooth_enabled` exclusion, this
    // falls back to the still-technically-connected Bluetooth session.
    server.release_session("peer-client", TransportKind::TcpWs, &server_db);

    assert_eq!(
        server.primary_transport("peer-client"),
        None,
        "a disabled pair's Bluetooth session must never win primary fallback, even while \
         it's still technically connected"
    );

    // The other half of the fix: `close_session_on` must actually tear the
    // session down, not just get excluded from primary selection forever
    // while the connection (and its ping/pong) keeps running underneath.
    sleep(Duration::from_millis(100)).await;
    assert!(
        !server.has_session_on("peer-client", TransportKind::Bluetooth),
        "the disabled pair's Bluetooth session must actually close, not just stop counting \
         toward primary"
    );
}

/// Regression test for a third P1 review finding on this PR: disabling
/// Bluetooth for a peer with *no pin at all* (the common case -- most
/// pairs are never pinned) must still recompute primary immediately, not
/// only in the pinned-to-Bluetooth case the sibling test above already
/// covers. Network is never brought up in this test at all, so Bluetooth
/// is primary purely by "it's the only thing connected" -- exactly the
/// unpinned scenario `device_connection_set_bluetooth_transport_with_
/// state_impl`'s own `disabling_a_bluetooth_pin` branch used to skip
/// calling `refresh_primary` for.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_unpinned_bluetooth_flips_primary_immediately() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "127.0.0.1");

    let (server, server_db) = server_state("transport-disable-unpinned-bluetooth-flips-primary");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some("127.0.0.1")),
            ))
            .execute(&mut conn)
            .expect("enable bluetooth for seeded peer");
    }

    let ble_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ble_port = ble_listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = ble_listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_server, gate_db).await;
    });
    sleep(Duration::from_millis(100)).await;

    let ble_stream = TcpStream::connect(("127.0.0.1", ble_port)).await.unwrap();
    let mut ble_link: Box<dyn Link> = Box::new(sim::SimLink::new(ble_stream));
    session::perform_client_auth(ble_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("bluetooth-kind auth should succeed for a bonded, enabled paired device");
    sleep(Duration::from_millis(50)).await;
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::Bluetooth),
        "bluetooth should be primary -- it's the only connected transport, and unpinned"
    );

    let mut conn = open_db_at_path(&server_db);
    crate::services::device_connection::device_connection_set_bluetooth_transport_with_state_impl(
        &mut conn,
        &server,
        crate::services::device_connection::types::DeviceBluetoothTransportInput {
            peer_device_id: "peer-client".to_string(),
            enabled: false,
            bluetooth_address: None,
        },
    )
    .expect("disable bluetooth");

    assert_eq!(
        server.primary_transport("peer-client"),
        None,
        "primary must be recomputed synchronously on disable even with no pin involved, not \
         left stale until an unrelated claim/release event happens to trigger a recompute"
    );
}

/// Regression test for a P1 review finding: `try_claim_session`'s
/// pre-lock `bluetooth_enabled` read is a time-of-check/time-of-use
/// window -- a disable landing between that read and the claim being
/// committed would otherwise let a Bluetooth session survive with a
/// stale "enabled" snapshot. `bluetooth_enabled` defaults to `false`
/// (the schema default, and every real disable ends there too), so
/// claiming with it never having been enabled at all exercises the same
/// post-commit self-correction path. Runs a minimal fake `run_session`
/// consumer (just enough to react to `SessionCommand::Close`, matching
/// what `close_session_on` actually needs downstream) since there's no
/// real link/gate in this test to drive one.
#[tokio::test(flavor = "multi_thread")]
async fn claiming_a_bluetooth_session_while_disabled_self_corrects() {
    let (server, server_db) = server_state("transport-claim-bluetooth-disabled-self-corrects");
    seed_paired_device(&server_db, "peer-client");

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let consumer_server = server.clone();
    let consumer_db = server_db.clone();
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            if matches!(command, crate::services::space_sync::types::SessionCommand::Close) {
                consumer_server.release_session("peer-client", TransportKind::Bluetooth, &consumer_db);
                break;
            }
        }
    });

    assert!(server.try_claim_session("peer-client", TransportKind::Bluetooth, tx, &server_db));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut closed = false;
    while tokio::time::Instant::now() < deadline {
        if !server.has_session_on("peer-client", TransportKind::Bluetooth) {
            closed = true;
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(
        closed,
        "a claim landing while bluetooth is disabled must self-correct, not survive"
    );
}

/// Regression test for a P1 review finding: `release_session` used to do
/// its (potentially slow, now-retrying) DB read *before* actually
/// removing the session from `peer_sessions`, so a dead session stayed
/// `has_session_on == true` for the whole retry window -- stalling
/// reconnect loops and leaving `push_to_peer` route into a closed link.
/// Points `db_path` at a directory that can never exist, so the trailing
/// DB read (which only feeds primary recompute, not the removal itself)
/// fails fast and predictably -- proving removal doesn't wait on it, and
/// wouldn't even if it hung or panicked.
#[tokio::test(flavor = "multi_thread")]
async fn release_session_removes_before_its_own_db_read_can_block_it() {
    let (server, server_db) = server_state("transport-release-removes-before-db-read");
    seed_paired_device(&server_db, "peer-client");
    let (tx, _rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", TransportKind::TcpWs, tx, &server_db));
    assert!(server.has_session_on("peer-client", TransportKind::TcpWs));

    let doomed_db_path = std::path::PathBuf::from("/nonexistent-directory-for-fini-tests/fini.db");
    let peer = "peer-client".to_string();
    let server_for_task = server.clone();
    let handle = tokio::spawn(async move {
        server_for_task.release_session(&peer, TransportKind::TcpWs, &doomed_db_path);
    });

    sleep(Duration::from_millis(20)).await;
    assert!(
        !server.has_session_on("peer-client", TransportKind::TcpWs),
        "removal must not wait on (or depend on the success of) the DB read that only feeds \
         primary recompute"
    );
    // Let the doomed background task finish (it panics on the unopenable
    // path once its own DB portion runs) without that panic failing this
    // test -- `JoinHandle::await` surfaces a panicking task as `Err`, not
    // as a propagated panic here.
    let _ = handle.await;
}

/// Regression test for a second P1 review finding on the same fix: with
/// removal now ordered before the DB read, `peer_primary_transport` must
/// never keep pointing at the just-removed session for the read's
/// duration (or forever, if it panics) -- `push_to_peer` reads that map
/// directly, so a dangling reference would silently stop all application
/// traffic even while a perfectly healthy *other* transport stays
/// connected. Points the DB read at the same doomed path as the sibling
/// test above.
#[tokio::test(flavor = "multi_thread")]
async fn release_session_reselects_primary_from_runtime_state_without_waiting_on_the_db() {
    let (server, server_db) = server_state("transport-release-reselects-primary");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set(paired_devices::bluetooth_enabled.eq(true))
            .execute(&mut conn)
            .expect("enable bluetooth for seeded peer");
    }

    let (tcp_tx, _tcp_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", TransportKind::TcpWs, tcp_tx, &server_db));
    let (ble_tx, _ble_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", TransportKind::Bluetooth, ble_tx, &server_db));

    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::TcpWs),
        "network wins by default with both connected"
    );

    let doomed_db_path = std::path::PathBuf::from("/nonexistent-directory-for-fini-tests/fini.db");
    let peer = "peer-client".to_string();
    let server_for_task = server.clone();
    let handle = tokio::spawn(async move {
        server_for_task.release_session(&peer, TransportKind::TcpWs, &doomed_db_path);
    });

    sleep(Duration::from_millis(20)).await;
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::Bluetooth),
        "primary must fail over to the still-connected Bluetooth session immediately, not \
         keep pointing at the just-removed Network one while the DB read is still (doomed to \
         be) in flight"
    );

    let _ = handle.await;
}

/// Regression test for a second P1 review finding on the DB-free
/// fallback: it must not pick a Bluetooth session that's already been
/// disabled but hasn't been torn down yet (`close_session_on`'s `Close`
/// is delivered asynchronously, so the session can still be sitting in
/// `peer_sessions` for a beat). Disables Bluetooth first (which updates
/// `peer_bluetooth_enabled_cache` immediately via the normal DB-backed
/// `refresh_primary` path), deliberately without draining the mailbox (no
/// `run_session` consumer exists in this test), then ends the *other*
/// transport against a doomed DB path to exercise the DB-free fallback
/// specifically.
#[tokio::test(flavor = "multi_thread")]
async fn reselect_primary_excludes_a_just_disabled_bluetooth_session() {
    let (server, server_db) = server_state("transport-reselect-excludes-disabled-bluetooth");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set(paired_devices::bluetooth_enabled.eq(true))
            .execute(&mut conn)
            .expect("enable bluetooth for seeded peer");
    }

    let (tcp_tx, _tcp_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", TransportKind::TcpWs, tcp_tx, &server_db));
    let (ble_tx, _ble_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", TransportKind::Bluetooth, ble_tx, &server_db));
    assert_eq!(server.primary_transport("peer-client"), Some(TransportKind::TcpWs));

    let mut conn = open_db_at_path(&server_db);
    crate::services::device_connection::device_connection_set_bluetooth_transport_with_state_impl(
        &mut conn,
        &server,
        crate::services::device_connection::types::DeviceBluetoothTransportInput {
            peer_device_id: "peer-client".to_string(),
            enabled: false,
            bluetooth_address: None,
        },
    )
    .expect("disable bluetooth");
    assert!(
        server.has_session_on("peer-client", TransportKind::Bluetooth),
        "the Bluetooth session must still be present -- its async Close hasn't been processed"
    );

    let doomed_db_path = std::path::PathBuf::from("/nonexistent-directory-for-fini-tests/fini.db");
    let peer = "peer-client".to_string();
    let server_for_task = server.clone();
    let handle = tokio::spawn(async move {
        server_for_task.release_session(&peer, TransportKind::TcpWs, &doomed_db_path);
    });

    sleep(Duration::from_millis(20)).await;
    assert_eq!(
        server.primary_transport("peer-client"),
        None,
        "the DB-free fallback must not pick the still-claimed but already-disabled Bluetooth \
         session as primary"
    );

    let _ = handle.await;
}

/// Regression test for Phase 1 of ADR 0002: whichever side of a network
/// session can read its own real Bluetooth address self-reports it via
/// `PeerFrame::BluetoothAddressUpdate`, once, right after auth. Here the
/// server side is configured (via the `FINI_LOCAL_BLUETOOTH_ADDRESS` test
/// escape hatch — real `bluetoothctl` isn't available/deterministic in
/// CI); the client reads it directly off the link rather than through a
/// full `run_session` loop, matching how these tests already only run
/// `run_session` on the accept side.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_is_sent_once_over_a_network_session() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-tcpws-self-report");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");

    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::BluetoothAddressUpdate { address })) => {
            assert_eq!(address, "AA:BB:CC:DD:EE:FF");
        }
        other => panic!("expected a BluetoothAddressUpdate frame, got {other:?}"),
    }

    std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");
}

/// Regression test: a peer that authenticates without reporting a
/// `protocol_version` (simulating a build from before `PROTOCOL_VERSION`
/// existed -- `perform_client_auth` always sends the current one, so this
/// hand-crafts the raw `Auth` frame instead) must never receive a
/// `BluetoothAddressUpdate`. That older peer's `PeerFrame` enum predates
/// the variant and would fail to decode it, dropping the whole
/// authenticated session -- exactly what version-gating this proactive
/// send exists to prevent.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_is_withheld_from_a_peer_that_reports_no_protocol_version() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-tcpws-self-report-old-peer");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");

    // Hand-crafted, deliberately omitting `protocol_version` -- this is
    // what an old build's `Auth` frame looked like before this field
    // existed. `#[serde(default)]` on the receiving end reads this as `0`.
    let old_style_auth = serde_json::json!({
        "type": "auth",
        "device_id": "peer-client",
        "peer_device_id": server.identity.device_id,
    });
    let plain = serde_json::to_vec(&old_style_auth).unwrap();
    let envelope = crate::services::transport::envelope::FrameEnvelope::new(
        crate::services::transport::envelope::EncScheme::None,
        plain,
    );
    let bytes = serde_json::to_vec(&envelope).unwrap();
    link.send(bytes).await.expect("send hand-crafted auth");

    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::AuthOk { .. })) => {}
        other => panic!("expected AuthOk, got {other:?}"),
    }

    match tokio::time::timeout(Duration::from_millis(300), recv_frame(link.as_mut())).await {
        Err(_) => {} // timed out waiting -- correctly withheld
        Ok(Some(Ok(PeerFrame::BluetoothAddressUpdate { .. }))) => {
            panic!("must not send BluetoothAddressUpdate to a peer reporting no protocol_version")
        }
        Ok(other) => panic!("unexpected frame while waiting: {other:?}"),
    }

    std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");
}

/// Regression test: the self-report must not be a one-shot fired only at
/// session start -- if the local Bluetooth controller changes underneath a
/// long-lived network session (simulated here by changing
/// `FINI_LOCAL_BLUETOOTH_ADDRESS` mid-session), the peer must eventually
/// learn the new address, not keep holding the stale one with no other way
/// to refresh it (this self-report only ever travels over the network
/// transport, so once network sync eventually breaks, a Bluetooth fallback
/// dial would be stuck targeting an address that no longer exists).
/// `FINI_BLUETOOTH_RECHECK_INTERVAL_MS` shortens the real 5-minute periodic
/// recheck so this can be observed deterministically.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_refreshes_when_the_local_address_changes_mid_session() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_RECHECK_INTERVAL_MS", "50");
    std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-tcpws-self-report-refresh");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");

    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::BluetoothAddressUpdate { address })) => {
            assert_eq!(address, "AA:BB:CC:DD:EE:FF");
        }
        other => panic!("expected the initial BluetoothAddressUpdate, got {other:?}"),
    }

    // Simulates a controller swap while this session stays live.
    std::env::set_var("FINI_LOCAL_BLUETOOTH_ADDRESS", "11:22:33:44:55:66");

    // The session's own app-level ping/ack loop (ADR-0003 revision) also
    // runs concurrently now -- skip past any incidental `Ping` (replying
    // `Pong`, same as a real peer would) while waiting for the refresh.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, recv_frame(link.as_mut())).await {
            Ok(Some(Ok(PeerFrame::BluetoothAddressUpdate { address }))) => {
                assert_eq!(address, "11:22:33:44:55:66");
                break;
            }
            Ok(Some(Ok(PeerFrame::Ping))) => {
                let _ = send_frame(link.as_mut(), &PeerFrame::Pong).await;
            }
            other => panic!("expected a refreshed BluetoothAddressUpdate after the address changed, got {other:?}"),
        }
    }

    std::env::remove_var("FINI_LOCAL_BLUETOOTH_ADDRESS");
    std::env::remove_var("FINI_BLUETOOTH_RECHECK_INTERVAL_MS");
}

/// Regression test for the receiving half of Phase 1: an inbound
/// `BluetoothAddressUpdate` for an already OS-paired address both persists
/// the address and auto-enables Bluetooth for that pair -- self-report
/// alone is sufficient confirmation only because it arrives over an
/// already-authenticated session, but auto-*enabling* additionally
/// requires OS bonding, mirroring `device_connection_set_bluetooth_transport_impl`'s
/// own precondition.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_persists_and_enables_when_os_paired() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-tcpws-self-report-enable");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    send_frame(
        link.as_mut(),
        &PeerFrame::BluetoothAddressUpdate {
            address: "aa:bb:cc:dd:ee:ff".to_string(),
        },
    )
    .await
    .expect("send self-report");

    sleep(Duration::from_millis(100)).await;
    let mut conn = open_db_at_path(&server_db);
    let row: (Option<String>, bool) = paired_devices::table
        .find("peer-client")
        .select((paired_devices::bluetooth_address, paired_devices::bluetooth_enabled))
        .first(&mut conn)
        .expect("load peer row");
    assert_eq!(row.0.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    assert!(row.1, "bluetooth should be auto-enabled when the reported address is OS-paired");

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
}

/// Mirror of the above without OS pairing: the address still gets stored
/// (so a later manual "Enable Bluetooth" click has something pre-filled),
/// but Bluetooth is not auto-enabled -- a self-report by itself proves
/// nothing about OS bonding.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_persists_without_enabling_when_not_os_paired() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();

    let (server, server_db) = server_state("transport-tcpws-self-report-no-enable");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    send_frame(
        link.as_mut(),
        &PeerFrame::BluetoothAddressUpdate {
            address: "11:22:33:44:55:66".to_string(),
        },
    )
    .await
    .expect("send self-report");

    sleep(Duration::from_millis(100)).await;
    let mut conn = open_db_at_path(&server_db);
    let row: (Option<String>, bool) = paired_devices::table
        .find("peer-client")
        .select((paired_devices::bluetooth_address, paired_devices::bluetooth_enabled))
        .first(&mut conn)
        .expect("load peer row");
    assert_eq!(row.0.as_deref(), Some("11:22:33:44:55:66"));
    assert!(!row.1, "bluetooth must not auto-enable without OS pairing");
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_ws_gate_rejects_unpaired_device() {
    let (server, _server_db) = server_state("transport-tcpws-reject");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server.db_path.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("unpaired device should be rejected");
    assert!(err.contains("auth rejected"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sim_gate_accepts_paired_device_and_claims_session_in_bluetooth_fallback_role() {
    let (server, server_db) = server_state("transport-sim-accept");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(sim::run_server(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = sim::dial(port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");

    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.primary_transport("peer-client"), Some(TransportKind::Sim));
}

/// ADR-0003 revision's core new guarantee: both Network and Bluetooth can
/// be simultaneously connected and claimed for the same peer -- Network
/// becomes primary (it wins whenever connected), but the Sim (playing
/// Bluetooth's role) session is not rejected or torn down; it stays live,
/// connected but not primary. This is what makes green a per-transport,
/// continuously-reproven property rather than something borrowed from
/// whichever session happens to be "the" one.
#[tokio::test(flavor = "multi_thread")]
async fn both_transports_can_be_simultaneously_connected_for_the_same_peer() {
    let (server, server_db) = server_state("transport-dual-connect");
    seed_paired_device(&server_db, "peer-client");
    let tcp_port = free_port().await;
    let sim_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        tcp_port,
    ));
    tokio::spawn(sim::run_server(server.clone(), server_db.clone(), sim_port));
    sleep(Duration::from_millis(100)).await;

    let mut first_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port)
        .await
        .expect("dial tcp_ws");
    session::perform_client_auth(first_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("first session should authenticate");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::TcpWs)
    );

    // A second connection on a *different* transport must be accepted, not
    // rejected -- the old sticky single-session invariant no longer holds.
    let mut second_link = sim::dial(sim_port).await.expect("dial sim");
    session::perform_client_auth(second_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("a session on a second transport must also be accepted");
    sleep(Duration::from_millis(50)).await;

    assert!(server.has_session_on("peer-client", TransportKind::TcpWs));
    assert!(server.has_session_on("peer-client", TransportKind::Sim));
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(TransportKind::TcpWs),
        "network stays primary even once bluetooth/sim also connects"
    );

    drop(first_link);
    drop(second_link);
}

/// Both adapters implement the same `Transport` port polymorphically — the
/// abstraction is real, not just declared.
#[tokio::test(flavor = "multi_thread")]
async fn both_adapters_satisfy_the_transport_port() {
    let (server, server_db) = server_state("transport-polymorphic");
    seed_paired_device(&server_db, "peer-client");
    let tcp_port = free_port().await;
    let sim_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        tcp_port,
    ));
    tokio::spawn(sim::run_server(server.clone(), server_db.clone(), sim_port));
    sleep(Duration::from_millis(100)).await;

    let adapters: Vec<(Box<dyn Transport>, u16, TransportKind)> = vec![
        (Box::new(tcp_ws::TcpWsTransport), tcp_port, TransportKind::TcpWs),
        (Box::new(sim::SimTransport), sim_port, TransportKind::Sim),
    ];

    for (adapter, port, expected_kind) in adapters {
        assert_eq!(adapter.kind(), expected_kind);
        let link = adapter
            .dial("peer-server", "127.0.0.1", port)
            .await
            .expect("dial via Transport trait object");
        assert_eq!(link.kind(), expected_kind);
    }
}

/// `device_connection::commands::send_pair_ws` is a one-shot sender
/// independent of `TcpWsLink` (connect, send one frame, close) — it does
/// not go through `Link::send`, so nothing structurally forces it to stay
/// wire-compatible with what `run_peer_gate`/`codec::decode_frame` expect
/// on the receiving end. This regression-tests that compatibility directly:
/// a real `PairRequest` sent via the production
/// `device_connection_send_pair_request_impl` path must be readable by a
/// real `tcp_ws` listener and land in the receiver's incoming-request queue.
#[tokio::test(flavor = "multi_thread")]
async fn send_pair_request_is_readable_by_the_receiving_gate() {
    use crate::services::device_connection::types::DevicePairRequestInput;
    use crate::services::device_connection::{
        device_connection_enter_add_mode_impl, device_connection_pair_incoming_requests_impl,
        device_connection_send_pair_request_impl,
    };

    let _add_mode_guard = super::ble::ADD_MODE_TEST_LOCK.lock().unwrap();
    let (receiver, receiver_db) = server_state("transport-send-pair-request-receiver");
    device_connection_enter_add_mode_impl(&receiver).expect("enter add mode");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        receiver.clone(),
        receiver_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let (sender, _sender_db) = server_state("transport-send-pair-request-sender");
    let sender_device_id = sender.identity.device_id.clone();
    let receiver_device_id = receiver.identity.device_id.clone();
    // `..._impl` uses `tauri::async_runtime::block_on` internally (matching
    // how a real, synchronous Tauri command runs); calling it directly from
    // this already-async test would panic ("runtime from within a
    // runtime"), so move it to a blocking thread like the real dispatcher does.
    tokio::task::spawn_blocking(move || {
        device_connection_send_pair_request_impl(
            &sender,
            DevicePairRequestInput {
                request_id: "req-1".to_string(),
                to_device_id: receiver_device_id,
                to_addr: "127.0.0.1".to_string(),
                to_ws_port: Some(port),
            },
        )
        .expect("send pair request");
    })
    .await
    .expect("join send-pair-request task");

    sleep(Duration::from_millis(200)).await;
    let incoming =
        device_connection_pair_incoming_requests_impl(&receiver).expect("list incoming requests");
    assert_eq!(incoming.len(), 1, "receiver should see the incoming pair request");
    assert_eq!(incoming[0].from_device_id, sender_device_id);
}

/// Full pairing round trip: request -> accept -> code delivered back to the
/// requester. Catches a specific regression class the previous test didn't:
/// `run_peer_gate` must capture the real peer address for a `PairRequest`
/// (matching the original `ws_server::handle_connection`'s
/// `stream.peer_addr()`), not an empty string — otherwise the accepter's
/// reply (`PairAccept`, addressed using that stored `from_addr`) fails to
/// parse a target IP and is silently never sent.
#[tokio::test(flavor = "multi_thread")]
async fn pair_request_accept_round_trip_delivers_a_code_back_to_the_requester() {
    use crate::services::device_connection::types::{DevicePairRequestAckInput, DevicePairRequestInput};
    use crate::services::device_connection::{
        device_connection_enter_add_mode_impl, device_connection_pair_accept_request_impl,
        device_connection_pair_incoming_requests_impl, device_connection_pair_outgoing_updates_impl,
        device_connection_send_pair_request_impl,
    };

    let _add_mode_guard = super::ble::ADD_MODE_TEST_LOCK.lock().unwrap();
    let requester_port = free_port().await;
    let accepter_port = free_port().await;
    // The requester's own port must match where its listener actually
    // binds: `device_connection_send_pair_request_impl` announces
    // `state.space_sync_ws_port` as the `from_ws_port` the accepter replies
    // to (`server_state`'s default would announce the crate's hardcoded
    // port instead, which may collide with an unrelated app already
    // running on the host).
    let (requester, requester_db) =
        server_state_on_port("transport-pair-round-trip-requester", requester_port);
    let (accepter, accepter_db) = server_state("transport-pair-round-trip-accepter");
    device_connection_enter_add_mode_impl(&requester).expect("enter add mode (requester)");
    device_connection_enter_add_mode_impl(&accepter).expect("enter add mode (accepter)");

    tokio::spawn(tcp_ws::run_server_on_port(
        requester.clone(),
        requester_db.clone(),
        requester_port,
    ));
    tokio::spawn(tcp_ws::run_server_on_port(
        accepter.clone(),
        accepter_db.clone(),
        accepter_port,
    ));
    sleep(Duration::from_millis(100)).await;

    let accepter_device_id = accepter.identity.device_id.clone();
    let requester_for_send = requester.clone();
    tokio::task::spawn_blocking(move || {
        device_connection_send_pair_request_impl(
            &requester_for_send,
            DevicePairRequestInput {
                request_id: "req-round-trip".to_string(),
                to_device_id: accepter_device_id,
                to_addr: "127.0.0.1".to_string(),
                to_ws_port: Some(accepter_port),
            },
        )
        .expect("send pair request");
    })
    .await
    .expect("join send-pair-request task");

    sleep(Duration::from_millis(200)).await;
    let incoming =
        device_connection_pair_incoming_requests_impl(&accepter).expect("list incoming requests");
    assert_eq!(incoming.len(), 1, "accepter should see the incoming pair request");
    // The regression this guards: without a real peer address, accepting
    // would fail before ever sending PairAccept.
    let accepter_for_accept = accepter.clone();
    tokio::task::spawn_blocking(move || {
        device_connection_pair_accept_request_impl(
            &accepter_for_accept,
            DevicePairRequestAckInput {
                request_id: "req-round-trip".to_string(),
            },
        )
        .expect("accept pair request")
    })
    .await
    .expect("join accept-pair-request task");

    sleep(Duration::from_millis(200)).await;
    let outgoing =
        device_connection_pair_outgoing_updates_impl(&requester).expect("list outgoing updates");
    assert_eq!(
        outgoing.len(),
        1,
        "requester should receive the pair code back from the accepter"
    );
    assert_eq!(outgoing[0].request_id, "req-round-trip");
    assert!(outgoing[0].code.chars().all(|ch| ch.is_ascii_digit()));
}

/// ADR-0003 revision: green is per-transport and earned by a bidirectional
/// `Ping`/`Pong` exchange, not borrowed from dial-failure history. A freshly
/// claimed session has no ack proof yet (amber, `AwaitingFirstAck`); once
/// `run_session`'s ping loop exchanges at least one round trip on a real
/// two-sided session, it becomes green (`transport_reliable`). Uses a short
/// `FINI_APP_PING_INTERVAL_MS`-independent wait since the loop's first tick
/// fires immediately.
#[tokio::test(flavor = "multi_thread")]
async fn a_freshly_claimed_session_starts_amber_and_becomes_green_once_pings_round_trip() {
    let (server, server_db) = server_state("transport-ping-ack-green");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;
    assert!(
        !server.transport_reliable("peer-client", TransportKind::TcpWs),
        "a freshly claimed session must not already be green"
    );
    // Regression test for a P1 review finding on this PR: the lightweight
    // live-poll surface (`device_connection_transport_liveness`) must
    // reflect this amber-not-green state too, not just `primary` -- the
    // whole point of it existing is to let a Bluetooth-only peer's row
    // (never covered by the network-presence-gated full poll) stay
    // current without the OS-bond-check cost.
    let liveness_before = crate::services::device_connection::device_connection_transport_liveness_impl(
        &server,
        "peer-client".to_string(),
    );
    let network_liveness_before = liveness_before
        .iter()
        .find(|l| l.kind == crate::services::device_connection::RowTransportKind::Network)
        .expect("network liveness row");
    assert!(network_liveness_before.connected);
    assert!(network_liveness_before.primary);
    assert!(
        network_liveness_before.code.is_some(),
        "must carry an amber code before the ping/ack proof completes, not None (green)"
    );

    // Drive the client side of the ping/ack exchange directly (this test
    // doesn't run a full peer-side `run_session` loop): reply to the
    // server's own Ping, and send one of our own for the server to ack.
    send_frame(link.as_mut(), &PeerFrame::Ping).await.expect("send ping");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut got_pong = false;
    let mut got_ping = false;
    while tokio::time::Instant::now() < deadline && !(got_pong && got_ping) {
        match tokio::time::timeout(Duration::from_millis(200), recv_frame(link.as_mut())).await {
            Ok(Some(Ok(PeerFrame::Pong))) => got_pong = true,
            Ok(Some(Ok(PeerFrame::Ping))) => {
                got_ping = true;
                let _ = send_frame(link.as_mut(), &PeerFrame::Pong).await;
            }
            _ => {}
        }
    }
    assert!(got_pong && got_ping, "expected a full bidirectional ping/ack round trip");

    sleep(Duration::from_millis(50)).await;
    assert!(
        server.transport_reliable("peer-client", TransportKind::TcpWs),
        "green once both directions of the ping/ack proof are complete"
    );

    let liveness_after = crate::services::device_connection::device_connection_transport_liveness_impl(
        &server,
        "peer-client".to_string(),
    );
    let network_liveness_after = liveness_after
        .iter()
        .find(|l| l.kind == crate::services::device_connection::RowTransportKind::Network)
        .expect("network liveness row");
    assert!(
        network_liveness_after.code.is_none(),
        "the lightweight live-poll surface must also report green (code: None) once the \
         ping/ack proof completes, not stay frozen at the pre-proof amber snapshot"
    );
}

// The mutual-dial race that `sim::should_dial_fallback_peer`'s deterministic
// dialer rule fixes is unit-tested directly there, mirroring
// `tcp_ws::should_dial_peer`'s own test — reproducing the actual network
// race end-to-end in an integration test proved unreliable (the exact
// collision needs both connects landing in the same async poll step, and a
// sleep-driven loopback test can't force that deterministically) without
// adding disproportionate complexity for what a pure function test already
// covers exactly.

/// Wraps a real link but reports a different `TransportKind` — lets these
/// tests drive `run_peer_gate`'s Bluetooth-specific enablement check using
/// `sim`'s real, already-proven TCP+length-delimited wire protocol, without
/// needing an actual BLE stack (no adapter's `Link` impl is swappable at the
/// `kind()` level otherwise).
struct AsBluetooth(Box<dyn Link>);

#[async_trait]
impl Link for AsBluetooth {
    fn kind(&self) -> TransportKind {
        TransportKind::Bluetooth
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        self.0.send(payload).await
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        self.0.recv().await
    }

    fn peer_addr(&self) -> Option<String> {
        self.0.peer_addr()
    }
}

/// Regression test for the gap Codex flagged on PR #140: `run_peer_gate`
/// used to authenticate any paired device regardless of its per-transport
/// enablement, so a peer that still had this pair's Bluetooth enabled on
/// their end could dial in and connect even after the local user disabled
/// Bluetooth for the pair. `bluetooth_enabled` defaults to `false`
/// (`specs/device-connect/README.md`: "disabled by default for every Fini
/// pair"), so a freshly paired, never-enabled device is exactly that case.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_gate_rejects_paired_device_with_bluetooth_disabled() {
    let (server, server_db) = server_state("transport-ble-gate-disabled");
    seed_paired_device(&server_db, "peer-client");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_server, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("a paired but bluetooth-disabled device must be rejected over a Bluetooth-kind link");
    assert!(err.contains("bluetooth disabled"), "unexpected error: {err}");
}

/// Mirror of the above with Bluetooth explicitly enabled for the pair: the
/// same link kind must now authenticate and claim the session as
/// `TransportKind::Bluetooth`, proving the new check only rejects the
/// disabled case rather than breaking Bluetooth accepts outright.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_gate_accepts_paired_device_with_bluetooth_enabled() {
    // `check_bluetooth_bond` requires the connecting link's `peer_addr()` to
    // match the pair's stored `bluetooth_address` *and* that address to be
    // OS-paired -- `sim::SimLink::peer_addr()` reports the TCP peer's IP
    // ("127.0.0.1" here), so that's what gets stored and allow-listed.
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "127.0.0.1");

    let (server, server_db) = server_state("transport-ble-gate-enabled");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some("127.0.0.1")),
            ))
            .execute(&mut conn)
            .expect("enable bluetooth for seeded peer");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_server, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("a bluetooth-enabled, bonded paired device should authenticate over a Bluetooth-kind link");

    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.primary_transport("peer-client"), Some(TransportKind::Bluetooth));

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
}

/// Regression test for the second gap Codex flagged on PR #140's re-review:
/// `check_bluetooth_enabled` alone proves the authenticated `device_id`'s
/// row has Bluetooth on, but says nothing about whether the central that
/// just connected is the specific bonded hardware the pairing metadata
/// expects. Here the pair's stored address is OS-paired, but a *different*
/// address connects (matching neither), so the accept must still be
/// rejected even though `bluetooth_enabled` is true.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_gate_rejects_when_connecting_address_does_not_match_the_bonded_address() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    // The allow-listed (OS-paired) address is real, but it isn't the one
    // stored for this pair, and it isn't the one connecting either -- only
    // "AA:BB:CC:DD:EE:FF" is both stored *and* what a real bonded device at
    // that address would present. "127.0.0.1" (what actually connects, via
    // SimLink) is deliberately left off the allow-list and mismatched from
    // storage, so this proves the address-match check independently of the
    // OS-paired check `bluetooth_gate_accepts_...` already covers.
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-ble-gate-address-mismatch");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set((
                paired_devices::bluetooth_enabled.eq(true),
                paired_devices::bluetooth_address.eq(Some("AA:BB:CC:DD:EE:FF")),
            ))
            .execute(&mut conn)
            .expect("enable bluetooth for seeded peer");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        // Connects as "127.0.0.1" (SimLink's real peer_addr), not the
        // stored "AA:BB:CC:DD:EE:FF" -- simulating a central that knows a
        // valid device_id but isn't the actual bonded hardware.
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_server, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("a connecting address that doesn't match the stored bond must be rejected");
    assert!(err.contains("not currently OS-paired"), "unexpected error: {err}");

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
}

/// ADR 0002 Phase 3: a `PairRequest` delivered over a Bluetooth-kind link
/// must be flagged `via_bluetooth`, with `from_bluetooth_address` set to the
/// address actually *observed* on that connection (`Link::peer_addr()`) --
/// trusted over any self-report, since the sender has no network endpoint
/// fields to self-report through this transport in the first place.
#[tokio::test(flavor = "multi_thread")]
async fn pair_request_over_a_bluetooth_link_captures_the_observed_address() {
    use crate::services::device_connection::types::PairRequestPayload;
    use crate::services::device_connection::{
        device_connection_enter_add_mode_impl, device_connection_pair_incoming_requests_impl,
        DISCOVERY_PROTOCOL,
    };

    let _add_mode_guard = super::ble::ADD_MODE_TEST_LOCK.lock().unwrap();
    let (receiver, receiver_db) = server_state("transport-pair-request-bluetooth");
    device_connection_enter_add_mode_impl(&receiver).expect("enter add mode");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_receiver = receiver.clone();
    let gate_db = receiver_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    send_frame(
        link.as_mut(),
        &PeerFrame::PairRequest(PairRequestPayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_request".to_string(),
            request_id: "req-ble-1".to_string(),
            from_device_id: "device-a".to_string(),
            from_hostname: "alpha".to_string(),
            from_discovery_port: None,
            from_ws_port: None,
            to_device_id: receiver.identity.device_id.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            expires_at: "2099-01-01T00:00:00Z".to_string(),
        }),
    )
    .await
    .expect("send pair request over bluetooth-kind link");

    sleep(Duration::from_millis(200)).await;
    let incoming =
        device_connection_pair_incoming_requests_impl(&receiver).expect("list incoming requests");
    assert_eq!(incoming.len(), 1);
    assert!(
        incoming[0].via_bluetooth,
        "a request delivered over a Bluetooth-kind link must be flagged as such"
    );
    assert_eq!(
        incoming[0].from_bluetooth_address.as_deref(),
        Some("127.0.0.1"),
        "must capture the address observed on the link itself"
    );
}

/// Mirror of the above for the completion leg: a `PairComplete` delivered
/// over a Bluetooth-kind link must trust the *observed* link address over
/// whatever the sender self-reported in the payload -- proven here by
/// deliberately mismatching them.
#[tokio::test(flavor = "multi_thread")]
async fn pair_complete_over_a_bluetooth_link_captures_the_observed_address() {
    use crate::services::device_connection::types::PairCompletePayload;
    use crate::services::device_connection::{
        device_connection_pair_outgoing_completions_impl, DISCOVERY_PROTOCOL,
    };

    let (receiver, receiver_db) = server_state("transport-pair-complete-bluetooth");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_receiver = receiver.clone();
    let gate_db = receiver_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    send_frame(
        link.as_mut(),
        &PeerFrame::PairComplete(PairCompletePayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_complete".to_string(),
            request_id: "req-ble-2".to_string(),
            from_device_id: "device-b".to_string(),
            from_hostname: "beta".to_string(),
            to_device_id: receiver.identity.device_id.clone(),
            paired_at: "2026-01-01T00:00:00Z".to_string(),
            bluetooth_address: Some("AA:BB:CC:DD:EE:FF".to_string()),
            key_material: None,
        }),
    )
    .await
    .expect("send pair complete over bluetooth-kind link");

    sleep(Duration::from_millis(200)).await;
    let completions = device_connection_pair_outgoing_completions_impl(&receiver)
        .expect("list outgoing completions");
    assert_eq!(completions.len(), 1);
    assert!(completions[0].via_bluetooth);
    assert_eq!(
        completions[0].bluetooth_address.as_deref(),
        Some("127.0.0.1"),
        "the observed link address must win over the payload's self-reported address"
    );
}

/// Mirror of the above for a network-carried completion: with no live
/// Bluetooth connection to observe an address from, the sender's
/// self-reported `PairCompletePayload::bluetooth_address` is what gets
/// captured instead (ADR 0002 Phase 3's "exchanges both transports'
/// details... regardless of which transport carried the pairing").
#[tokio::test(flavor = "multi_thread")]
async fn pair_complete_over_network_uses_the_self_reported_bluetooth_address() {
    use crate::services::device_connection::types::PairCompletePayload;
    use crate::services::device_connection::{
        device_connection_pair_outgoing_completions_impl, DISCOVERY_PROTOCOL,
    };

    let (receiver, receiver_db) = server_state("transport-pair-complete-network-btaddr");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        receiver.clone(),
        receiver_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    send_frame(
        link.as_mut(),
        &PeerFrame::PairComplete(PairCompletePayload {
            protocol: DISCOVERY_PROTOCOL.to_string(),
            kind: "pair_complete".to_string(),
            request_id: "req-net-1".to_string(),
            from_device_id: "device-c".to_string(),
            from_hostname: "gamma".to_string(),
            to_device_id: receiver.identity.device_id.clone(),
            paired_at: "2026-01-01T00:00:00Z".to_string(),
            bluetooth_address: Some("11:22:33:44:55:66".to_string()),
            key_material: None,
        }),
    )
    .await
    .expect("send pair complete over network");

    sleep(Duration::from_millis(200)).await;
    let completions = device_connection_pair_outgoing_completions_impl(&receiver)
        .expect("list outgoing completions");
    assert_eq!(completions.len(), 1);
    assert!(!completions[0].via_bluetooth);
    assert_eq!(
        completions[0].bluetooth_address.as_deref(),
        Some("11:22:33:44:55:66")
    );
}

/// Regression test: `BluetoothProbe` ("Find via Bluetooth"'s confirmation
/// step) must succeed for a paired peer whose Bluetooth transport is *not*
/// enabled yet -- that's the normal case this discovery flow exists for.
/// Before this fix, `find_peer_address` reused the ordinary Auth/AuthOk
/// handshake, whose `check_bluetooth_enabled` precondition made this
/// scenario impossible to ever complete.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_probe_confirms_a_paired_device_even_when_bluetooth_is_not_yet_enabled() {
    let (receiver, receiver_db) = server_state("transport-bluetooth-probe-not-enabled");
    seed_paired_device(&receiver_db, "peer-client");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_receiver = receiver.clone();
    let gate_db = receiver_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    send_frame(
        link.as_mut(),
        &PeerFrame::BluetoothProbe {
            device_id: "peer-client".to_string(),
        },
    )
    .await
    .expect("send bluetooth probe");

    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::BluetoothProbeReply { device_id })) => {
            assert_eq!(device_id, receiver.identity.device_id);
        }
        other => panic!("expected a BluetoothProbeReply, got {other:?}"),
    }
}

/// Regression test for the P2 review finding: a probe from a paired but
/// *explicitly disabled* peer must get no reply either -- replying would
/// let that peer believe "Find via Bluetooth" succeeded and persist/enable
/// the address on its own end, only for every real session attempt to
/// then be rejected by this device's own `check_bluetooth_enabled` gate.
/// Distinct from the "not yet enabled" case above: that one must still
/// reply (it's the whole point of this discovery flow), an explicit
/// disable must not.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_probe_gets_no_reply_when_explicitly_disabled() {
    let (receiver, receiver_db) = server_state("transport-bluetooth-probe-disabled");
    seed_paired_device(&receiver_db, "peer-client");
    {
        let mut conn = open_db_at_path(&receiver_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set(paired_devices::bluetooth_disabled_by_user.eq(true))
            .execute(&mut conn)
            .expect("mark the pair as explicitly disabled");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_receiver = receiver.clone();
    let gate_db = receiver_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    send_frame(
        link.as_mut(),
        &PeerFrame::BluetoothProbe {
            device_id: "peer-client".to_string(),
        },
    )
    .await
    .expect("send bluetooth probe");

    match recv_frame(link.as_mut()).await {
        None | Some(Err(_)) => {}
        other => panic!("an explicitly disabled pair must not reply to BluetoothProbe, got {other:?}"),
    }
}

/// Mirror of the above: a probe from a device_id that isn't actually paired
/// must get no reply at all -- same "silently ignore, don't confirm/deny"
/// pattern `DiscoveryHello` uses.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_probe_gets_no_reply_from_an_unpaired_device_id() {
    let (receiver, receiver_db) = server_state("transport-bluetooth-probe-unpaired");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_receiver = receiver.clone();
    let gate_db = receiver_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn Link> = Box::new(AsBluetooth(Box::new(sim::SimLink::new(stream))));
        session::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
    send_frame(
        link.as_mut(),
        &PeerFrame::BluetoothProbe {
            device_id: "a-stranger".to_string(),
        },
    )
    .await
    .expect("send bluetooth probe");

    match recv_frame(link.as_mut()).await {
        None | Some(Err(_)) => {}
        other => panic!("an unpaired probe must not get a reply, got {other:?}"),
    }
}

/// Regression test for Phase 3 of ADR 0002: `DiscoveryHello` only gets a
/// reply when the receiver is actually in add-mode -- the BLE-scan
/// equivalent of network discovery simply not broadcasting outside
/// add-mode. Uses `set_add_mode_for_test` (instance-scoped) rather than the
/// real `enter_add_mode_impl`, which would also flip the process-global
/// `transport::ble` advertising flag `ble::tests` already covers
/// separately.
#[tokio::test(flavor = "multi_thread")]
async fn discovery_hello_gets_a_reply_only_when_the_receiver_is_in_add_mode() {
    let (server, server_db) = server_state("transport-discovery-hello-on");
    server.set_add_mode_for_test(true);
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    send_frame(link.as_mut(), &PeerFrame::DiscoveryHello)
        .await
        .expect("send discovery hello");
    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::DiscoveryHelloReply { device_id, hostname })) => {
            assert_eq!(device_id, server.identity.device_id);
            assert_eq!(hostname, server.identity.hostname);
        }
        other => panic!("expected a DiscoveryHelloReply while in add-mode, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_hello_gets_no_reply_when_the_receiver_is_not_in_add_mode() {
    let (server, server_db) = server_state("transport-discovery-hello-off");
    // Add-mode is off by default -- no set_add_mode_for_test call.
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        port,
    ));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    send_frame(link.as_mut(), &PeerFrame::DiscoveryHello)
        .await
        .expect("send discovery hello");
    // The server task returns without replying, dropping its side of the
    // link -- observed here as either a clean EOF (`None`) or a connection
    // error from the abrupt close (`Some(Err(_))`), depending on how the
    // underlying transport surfaces an ungraceful drop. Either is "no
    // valid reply was given"; only an actual `DiscoveryHelloReply` fails
    // the test.
    match recv_frame(link.as_mut()).await {
        None | Some(Err(_)) => {}
        other => panic!("a receiver not in add-mode must not reply to DiscoveryHello, got {other:?}"),
    }
}
