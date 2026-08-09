//! End-to-end proof that the transport abstraction works: two independent
//! adapters (`tcp_ws`, `sim`) carry the exact same `session::run_peer_gate`/
//! `run_session` engine, the sticky single-session invariant holds across
//! them, and both satisfy the `Transport` trait polymorphically. This is
//! the protocol-level coverage referenced by the E2E topology matrix in
//! `specs/e2e/transports.md` — it proves selection/fallback/handoff
//! semantics without needing a real Android runtime or a real radio.

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

/// `FINI_BLUETOOTH_PAIRED_ADDRESSES` is process-global too — same reasoning,
/// separate lock since it guards a disjoint set of tests. Mirrors the lock
/// of the same name in `device_connection::commands::tests`.
static BLUETOOTH_ADDRESS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        server.session_kind("peer-client"),
        Some(TransportKind::TcpWs)
    );
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
    assert_eq!(server.session_kind("peer-client"), Some(TransportKind::Sim));
}

/// The core handoff-safety guarantee: at most one authenticated session can
/// ever be live for a peer, regardless of which transport it arrives on.
/// This is what makes duplicated/lost sync events structurally impossible
/// (`specs/space-sync/README.md`).
#[tokio::test(flavor = "multi_thread")]
async fn sticky_single_session_rejects_a_concurrent_second_claim() {
    let (server, server_db) = server_state("transport-sticky");
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

    // First session claims via TcpWs and is kept open (link held, not dropped).
    let mut first_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port)
        .await
        .expect("dial tcp_ws");
    session::perform_client_auth(first_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("first session should authenticate");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        server.session_kind("peer-client"),
        Some(TransportKind::TcpWs)
    );

    // Second attempt, over Sim, must be rejected while the first is live —
    // sticky handoff means no mid-session transport migration.
    let mut second_link = sim::dial(sim_port).await.expect("dial sim");
    let err = session::perform_client_auth(
        second_link.as_mut(),
        "peer-client",
        &server.identity.device_id,
    )
    .await
    .expect_err("second concurrent session must be rejected");
    assert!(err.contains("session already active"));

    // The first session is still the one on record.
    assert_eq!(
        server.session_kind("peer-client"),
        Some(TransportKind::TcpWs)
    );
    drop(first_link);
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

/// Regression test: discovery presence alone must not make selection treat
/// a peer as network-available forever. `network_peer_available` only
/// reflects that beacons are arriving; `tcp_ws::dial_with_backoff` retries
/// a connect/auth failure indefinitely without giving up as long as
/// presence holds, so without `network_effectively_available` factoring in
/// bounded failures, `sim::spawn_fallback_dial_loop` would never engage for
/// a peer whose WebSocket port is permanently unreachable (bind failure,
/// firewall) but who is still discoverable.
#[test]
fn network_effectively_available_demotes_after_repeated_tcp_failures() {
    use crate::services::transport::selection::NETWORK_UNRESPONSIVE_THRESHOLD;

    let (state, _db) = server_state("transport-network-effectively-available");

    // No presence at all: never effectively available regardless of failures
    // (this test can't inject synthetic presence — that's owned by the
    // discovery worker's internal state — so it verifies the failure-count
    // threshold mechanics directly; presence itself is exercised end-to-end
    // by the existing discovery/pairing tests).
    assert!(!state.network_effectively_available("peer-x"));

    for _ in 0..(NETWORK_UNRESPONSIVE_THRESHOLD - 1) {
        state.record_tcp_dial_failure("peer-x");
    }
    assert_eq!(
        state.tcp_dial_failure_count("peer-x"),
        NETWORK_UNRESPONSIVE_THRESHOLD - 1,
        "below threshold yet"
    );

    state.record_tcp_dial_failure("peer-x");
    assert_eq!(
        state.tcp_dial_failure_count("peer-x"),
        NETWORK_UNRESPONSIVE_THRESHOLD,
        "at threshold"
    );

    // A later success resets the counter (transient blip, not permanent).
    state.record_tcp_dial_success("peer-x");
    assert_eq!(state.tcp_dial_failure_count("peer-x"), 0);
}

/// Regression test: `tcp_dial_failures` must not outlive the Sim session it
/// caused. `tcp_ws::dial_with_backoff` stops updating the counter the
/// instant any session exists (it gives up as soon as `has_session` is
/// true), so once a Sim fallback session claims, the failure count is
/// frozen at whatever it was — stale by the time that session eventually
/// ends. Without resetting it there, the *next* establishment cycle would
/// see a stale "network unresponsive" verdict and start a fresh Sim
/// fallback dial concurrently with a fresh tcp_ws dial, racing for the
/// claim despite the underlying network condition being unknown/possibly
/// recovered — contrary to "the next session selection returns to
/// network-first order" (`specs/space-sync/README.md`).
#[tokio::test(flavor = "multi_thread")]
async fn tcp_failure_count_resets_after_a_sim_session_ends() {
    let (dialer, dialer_db) = server_state("transport-sim-failure-reset-dialer");
    let peer_id = "peer-fake-server".to_string();
    seed_paired_device(&dialer_db, &peer_id);

    // Pre-seed failures as if tcp_ws had already given up on this peer,
    // triggering the Sim fallback role.
    for _ in 0..(3 + 2) {
        dialer.record_tcp_dial_failure(&peer_id);
    }
    assert!(dialer.tcp_dial_failure_count(&peer_id) >= 3);

    // A minimal fake "server": accept one connection, speak just enough of
    // the peer protocol to authenticate, then close — simulating the Sim
    // session ending shortly after establishing, without needing the full
    // sim::run_server accept loop (this test only has one dialer, so there's
    // no mutual-dial race to worry about — see should_dial_fallback_peer's
    // own direct test for that).
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let mut link: Box<dyn Link> = Box::new(sim::SimLink::new(stream));
        if let Some(Ok(PeerFrame::Auth { .. })) = recv_frame(link.as_mut()).await {
            let _ = send_frame(link.as_mut(), &PeerFrame::AuthOk).await;
        }
        // link drops here, closing the connection right after handshake.
    });

    std::env::set_var("FINI_SIM_PEER_PORTS", port.to_string());
    let paired_peer_ids: std::collections::HashSet<String> = [peer_id.clone()].into_iter().collect();
    sim::spawn_fallback_dial_loop(&dialer, dialer_db.clone(), &paired_peer_ids);
    std::env::remove_var("FINI_SIM_PEER_PORTS");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut reset = false;
    while tokio::time::Instant::now() < deadline {
        if dialer.tcp_dial_failure_count(&peer_id) == 0 {
            reset = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        reset,
        "tcp_dial_failures should be cleared once the Sim session it caused ends"
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
    assert_eq!(server.session_kind("peer-client"), Some(TransportKind::Bluetooth));

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
