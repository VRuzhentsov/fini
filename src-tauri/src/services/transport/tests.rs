//! End-to-end proof that the transport abstraction works: two independent
//! adapters (`tcp_ws`, `sim`) carry the exact same `session::run_peer_gate`/
//! `run_session` engine, the sticky single-session invariant holds across
//! them, and both satisfy the `Transport` trait polymorphically. This is
//! the protocol-level coverage referenced by the E2E topology matrix in
//! `specs/e2e/transports.md` — it proves selection/fallback/handoff
//! semantics without needing a real Android runtime or a real radio.

use std::path::PathBuf;
use std::time::Duration;

use diesel::prelude::*;
use tokio::net::TcpListener;
use tokio::time::sleep;

use crate::models::CreatePairedDeviceInput;
use crate::schema::paired_devices;
use crate::services::db::{open_db_at_path, temp_db_path};
use crate::services::device_connection::DeviceConnectionState;
use crate::services::space_sync::session;
use crate::services::transport::{sim, tcp_ws, Transport, TransportKind};

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
