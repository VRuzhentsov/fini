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
        server.session_kind("peer-client"),
        Some(TransportKind::TcpWs)
    );
}

/// Regression test for ADR-0003 Phase 3's core new mechanism: before this,
/// nothing could ever end a live `run_session` loop except the loop itself
/// noticing its own transport failed. `request_session_close` is the first
/// external caller of the mailbox's new `SessionCommand::Close` -- proves
/// it actually reaches and terminates a genuinely live session, not just
/// that the method compiles.
#[tokio::test(flavor = "multi_thread")]
async fn request_session_close_terminates_a_live_session() {
    let (server, server_db) = server_state("transport-request-close");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;
    assert!(server.has_session("peer-client"), "session must be live before closing it");

    assert!(
        server.request_session_close("peer-client"),
        "must find a mailbox to send Close through"
    );
    sleep(Duration::from_millis(50)).await;
    assert!(
        !server.has_session("peer-client"),
        "Close must actually terminate run_session's loop, not just be accepted into the mailbox"
    );
}

/// ADR-0003 Phase 3's full command: pinning a peer to a transport different
/// from the one it's currently live on (a) persists the preference and (b)
/// sends the peer PeerFrame::SwitchTransport. Deliberately does not itself
/// assert the session then closes -- a P1 finding caught that closing
/// unilaterally on this side's own guess (even though it's a fresh,
/// almost-certainly-winning timestamp) can strand a session if the peer
/// rejects for a reason this side can't predict (e.g. Bluetooth disabled on
/// the peer's end); actually closing is the *peer's* own adoption's job
/// (see `session::handle_inbound`), which this raw test client doesn't
/// exercise.
#[tokio::test(flavor = "multi_thread")]
async fn set_preferred_transport_persists_and_notifies_a_mismatched_session() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-set-preferred-mismatch");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.session_kind("peer-client"), Some(TransportKind::TcpWs));

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

    // The frame must arrive on the wire before this side tears the
    // connection down underneath it. On a machine with a real Bluetooth
    // controller, `run_session` also unconditionally self-reports
    // `PeerFrame::BluetoothAddressUpdate` right after auth (independent of
    // this test's own concerns) -- skip past any such frames rather than
    // asserting on exact frame order between two independent features.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for SwitchTransport");
        match recv_frame(link.as_mut()).await {
            Some(Ok(PeerFrame::SwitchTransport { to, .. })) => {
                assert_eq!(to, TransportKind::Bluetooth);
                break;
            }
            Some(Ok(PeerFrame::BluetoothAddressUpdate { .. })) => continue,
            other => panic!("expected SwitchTransport, got {other:?}"),
        }
    }

    sleep(Duration::from_millis(50)).await;
    assert!(
        server.has_session("peer-client"),
        "must not close unilaterally on a guess -- only the peer's own adoption \
         (not exercised by this raw test client) decides that"
    );
}

/// Sibling of the test above: pinning a peer to the transport it's *already*
/// live on must not disturb the session at all -- no reason to tear down
/// and immediately re-establish the exact same connection.
#[tokio::test(flavor = "multi_thread")]
async fn set_preferred_transport_leaves_an_already_matching_session_alone() {
    let (server, server_db) = server_state("transport-set-preferred-match");
    seed_paired_device(&server_db, "peer-client");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.session_kind("peer-client"), Some(TransportKind::TcpWs));

    let mut conn = open_db_at_path(&server_db);
    crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(TransportKind::TcpWs),
    )
    .expect("set preferred transport");

    sleep(Duration::from_millis(50)).await;
    assert!(
        server.has_session("peer-client"),
        "pinning the already-live transport must not close the session"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: a peer on
/// an older build (protocol version 1, before `PeerFrame::SwitchTransport`
/// existed) cannot decode the frame -- sending it and force-closing the
/// session anyway would just have that peer reconnect over whatever
/// transport it still understands (Network), silently undoing the pin the
/// moment it lands. Neither the frame nor the close should happen; the
/// preference is still persisted so this device's own dial loops (and a
/// future reconnect after the peer upgrades) still honor it.
#[tokio::test(flavor = "multi_thread")]
async fn set_preferred_transport_withholds_switch_from_a_peer_on_an_old_protocol_version() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-set-preferred-old-peer");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");

    // Hand-crafted Auth declaring protocol_version 1 -- a build with Phase
    // 1/2 (ping/pong, the unified status model) but not yet Phase 3's
    // SwitchTransport, mirroring the existing no-protocol-version test's
    // hand-crafted-frame pattern above.
    let old_auth = serde_json::json!({
        "type": "auth",
        "device_id": "peer-client",
        "peer_device_id": server.identity.device_id,
        "protocol_version": 1,
    });
    let plain = serde_json::to_vec(&old_auth).unwrap();
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
    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.session_kind("peer-client"), Some(TransportKind::TcpWs));

    let mut conn = open_db_at_path(&server_db);
    let updated = crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(TransportKind::Bluetooth),
    )
    .expect("set preferred transport");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    assert_eq!(
        updated.preferred_transport.as_deref(),
        Some("bluetooth"),
        "the preference must still persist even though the live peer can't be notified"
    );

    // On a machine with a real Bluetooth controller, `run_session` also
    // self-reports `PeerFrame::BluetoothAddressUpdate` right after auth for
    // any peer reporting protocol_version >= 1 -- unrelated to this test's
    // own concern, so skip past it the same way the mismatch test above does.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, recv_frame(link.as_mut())).await {
            Err(_) => break, // timed out waiting -- correctly withheld
            Ok(Some(Ok(PeerFrame::BluetoothAddressUpdate { .. }))) => continue,
            Ok(other) => panic!("must not send SwitchTransport to a peer on protocol version 1, got {other:?}"),
        }
    }

    assert!(
        server.has_session("peer-client"),
        "must not force-close a session the peer can't be told to reconnect correctly around"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: a pin set
/// while the peer isn't connected has nothing to notify or close at that
/// moment, but the *next* session that establishes on the wrong transport
/// -- inbound here -- must still relay the correct pin, rather than going
/// live and staying live indefinitely with nobody ever told about the pin
/// (nothing else ever revisits an already-claimed session against the
/// preference on its own). Unlike an outright reject, the connection is
/// accepted normally (`AuthOk`) so the relay actually has a channel to
/// travel over. Deliberately does *not* assert the session then closes:
/// closing unconditionally on a guess that could itself be the stale one
/// is exactly the bug a later finding caught (see
/// `offline_pin_converges_the_deterministic_dialer_via_the_first_session_established`'s
/// doc comment) -- actually closing is `handle_inbound`'s job, once the
/// peer's own adoption settles which side's preference wins.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_ws_gate_relays_without_closing_an_inbound_session_that_mismatches_the_local_transport_pin()
{
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-gate-preference-mismatch");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");

    let mut conn = open_db_at_path(&server_db);
    crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(TransportKind::Bluetooth),
    )
    .expect("set preferred transport ahead of any connection");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("the connection must be accepted normally, not rejected");

    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::SwitchTransport { to, .. })) => assert_eq!(to, TransportKind::Bluetooth),
        other => panic!("expected SwitchTransport, got {other:?}"),
    }

    sleep(Duration::from_millis(50)).await;
    assert!(
        server.has_session("peer-client"),
        "must not close unilaterally on a guess that could itself be the stale side -- \
         only the peer's own adoption (not exercised by this raw test client) decides that"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: when a peer
/// with the higher device ID pins itself to Bluetooth while offline, it
/// never dials at all (see `should_dial_peer`'s deterministic-dialer
/// election) -- the *lower*-ID side is the only one that can ever make
/// progress, and it doesn't yet know about the new pin (nothing could
/// deliver it while offline). Its own stale "network" pin would otherwise
/// block Bluetooth fallback forever too (`ble.rs`'s own sticky-pin check).
/// Proves the actual fix: the first TCP session that manages to establish
/// at all relays the peer's pin and closes (previous test), and the dialer
/// receiving that relay adopts it into its own preference -- converging
/// without either side ever needing to reject or give up.
#[tokio::test(flavor = "multi_thread")]
async fn offline_pin_converges_the_deterministic_dialer_via_the_first_session_established() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (responder, responder_db) = server_state("transport-offline-pin-converge-responder");
    let (dialer, dialer_db) = server_state("transport-offline-pin-converge-dialer");
    seed_paired_device(&responder_db, &dialer.identity.device_id);
    seed_paired_device(&dialer_db, &responder.identity.device_id);
    // Both sides had already bonded over Bluetooth at some point -- the
    // dialer just doesn't yet know the responder now *prefers* it. The
    // responder needs this on its own side too, to satisfy
    // `device_connection_set_preferred_transport_impl`'s own current-
    // eligibility check when pinning itself below; the dialer needs it on
    // its side to satisfy `adopt_peer_transport_preference`'s check once
    // the relay reaches it.
    seed_bluetooth_enabled_peer(&responder_db, &dialer.identity.device_id, "AA:BB:CC:DD:EE:FF");
    diesel::update(paired_devices::table.find(&responder.identity.device_id))
        .set(paired_devices::bluetooth_enabled.eq(true))
        .execute(&mut open_db_at_path(&dialer_db))
        .expect("enable bluetooth for the responder on the dialer's side");

    let mut conn = open_db_at_path(&responder_db);
    crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut conn,
        &responder,
        dialer.identity.device_id.clone(),
        Some(TransportKind::Bluetooth),
    )
    .expect("pin the responder to Bluetooth while offline");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(responder.clone(), responder_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", port);
    tokio::spawn(tcp_ws::dial_with_backoff(
        dialer.clone(),
        dialer_db.clone(),
        responder.identity.device_id.clone(),
        "127.0.0.1".to_string(),
        port,
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut converged = false;
    while tokio::time::Instant::now() < deadline {
        let mut dialer_conn = open_db_at_path(&dialer_db);
        if crate::services::device_connection::peer_transport_preference(
            &mut dialer_conn,
            &responder.identity.device_id,
        ) == Some("bluetooth".to_string())
        {
            converged = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        converged,
        "the dialer must adopt the responder's offline pin once the relay reaches it, \
         rather than being stuck retrying Network forever"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: when *both*
/// sides set conflicting pins while offline (dialer -> "network", older;
/// responder -> "bluetooth", newer), the session that establishes on the
/// dialer's own preferred transport (Network -- its own pin doesn't block
/// that) still carries the responder's *newer* preference in, and the
/// dialer must adopt it and close, not just keep the session because its
/// own stale guess happened to match what's live. A design that
/// unconditionally closed a "mismatch" on the *responder's* own initiative
/// (the previous version of this fix) would instead have the responder
/// close every time this session re-establishes, while the dialer's own
/// stale pin keeps re-offering Network -- neither side ever winning,
/// forever. This proves that no longer happens: the dialer's preference
/// converges to the responder's newer one.
#[tokio::test(flavor = "multi_thread")]
async fn conflicting_offline_pins_converge_on_the_newer_one_without_a_reconnect_loop() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (responder, responder_db) = server_state("transport-conflicting-pins-responder");
    let (dialer, dialer_db) = server_state("transport-conflicting-pins-dialer");
    seed_paired_device(&responder_db, &dialer.identity.device_id);
    seed_paired_device(&dialer_db, &responder.identity.device_id);
    seed_bluetooth_enabled_peer(&responder_db, &dialer.identity.device_id, "AA:BB:CC:DD:EE:FF");
    diesel::update(paired_devices::table.find(&responder.identity.device_id))
        .set(paired_devices::bluetooth_enabled.eq(true))
        .execute(&mut open_db_at_path(&dialer_db))
        .expect("enable bluetooth for the responder on the dialer's side");

    // Dialer's own (older, soon-to-be-stale) pin. `utc_now()` only has
    // second-level precision, so a short real sleep can't reliably
    // guarantee a strictly earlier timestamp than the responder's pin
    // below -- backdate it explicitly instead.
    let mut dialer_conn = open_db_at_path(&dialer_db);
    crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut dialer_conn,
        &dialer,
        responder.identity.device_id.clone(),
        Some(TransportKind::TcpWs),
    )
    .expect("pin the dialer to Network while offline");
    diesel::update(paired_devices::table.find(&responder.identity.device_id))
        .set(paired_devices::preferred_transport_set_at.eq(Some("2020-01-01T00:00:00Z")))
        .execute(&mut dialer_conn)
        .expect("backdate the dialer's own pin");

    // Responder's own (newer, winning) pin.
    let mut responder_conn = open_db_at_path(&responder_db);
    crate::services::device_connection::device_connection_set_preferred_transport_impl(
        &mut responder_conn,
        &responder,
        dialer.identity.device_id.clone(),
        Some(TransportKind::Bluetooth),
    )
    .expect("pin the responder to Bluetooth while offline");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(responder.clone(), responder_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", port);
    tokio::spawn(tcp_ws::dial_with_backoff(
        dialer.clone(),
        dialer_db.clone(),
        responder.identity.device_id.clone(),
        "127.0.0.1".to_string(),
        port,
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut converged = false;
    while tokio::time::Instant::now() < deadline {
        let mut conn = open_db_at_path(&dialer_db);
        if crate::services::device_connection::peer_transport_preference(
            &mut conn,
            &responder.identity.device_id,
        ) == Some("bluetooth".to_string())
        {
            converged = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        converged,
        "the dialer's own stale Network pin must yield to the responder's newer Bluetooth one"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: a
/// Bluetooth pin must not block Network *forever* once Bluetooth has
/// demonstrably become permanently unreachable for this peer (disabled on
/// the peer's end, repeatedly rejecting every connect attempt) -- sticky
/// only means "don't second-guess on a transient blip," not "never
/// reconsider even after every alternative also stops working." Proves
/// `tcp_ws::dial_with_backoff`'s own override: once
/// `bluetooth_dial_failure_count` has crossed the same threshold that
/// already demotes automatic selection, a Bluetooth pin no longer
/// suppresses the Network dial loop, and a real Network session
/// establishes despite the pin.
#[tokio::test(flavor = "multi_thread")]
async fn stale_bluetooth_pin_no_longer_blocks_network_once_bluetooth_repeatedly_fails() {
    use crate::services::transport::selection::TRANSPORT_UNRESPONSIVE_THRESHOLD;

    let (responder, responder_db) = server_state("transport-stale-bluetooth-pin-responder");
    let (dialer, dialer_db) = server_state("transport-stale-bluetooth-pin-dialer");
    seed_paired_device(&responder_db, &dialer.identity.device_id);
    seed_paired_device(&dialer_db, &responder.identity.device_id);

    let mut dialer_conn = open_db_at_path(&dialer_db);
    diesel::update(paired_devices::table.find(&responder.identity.device_id))
        .set(paired_devices::preferred_transport.eq(Some("bluetooth")))
        .execute(&mut dialer_conn)
        .expect("pin the dialer to bluetooth directly (bypassing the impl's own eligibility check)");

    for _ in 0..TRANSPORT_UNRESPONSIVE_THRESHOLD {
        dialer.record_bluetooth_dial_failure(&responder.identity.device_id);
    }
    assert!(
        !dialer.bluetooth_effectively_reliable(&responder.identity.device_id),
        "test setup: bluetooth must look unreliable before dial_with_backoff is exercised"
    );

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(responder.clone(), responder_db.clone(), port));
    sleep(Duration::from_millis(100)).await;
    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", port);

    tokio::spawn(tcp_ws::dial_with_backoff(
        dialer.clone(),
        dialer_db,
        responder.identity.device_id.clone(),
        "127.0.0.1".to_string(),
        port,
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut established = false;
    while tokio::time::Instant::now() < deadline {
        if dialer.session_kind(&responder.identity.device_id) == Some(TransportKind::TcpWs) {
            established = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        established,
        "a Bluetooth pin that's demonstrably unreliable must not suppress Network forever"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: a stale
/// "configured" row (`DeviceView`'s polling only refreshes session
/// liveness, not full eligibility -- see the frontend's own
/// `refreshLiveConnectedState` doc comment) can stay clickable well after
/// the OS Bluetooth bond quietly disappears. `device_connection_set_
/// preferred_transport_impl` must re-validate current eligibility itself
/// rather than trusting the click -- persisting and announcing a pin that
/// can never actually connect just relocates the stranding hazard instead
/// of preventing it.
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

/// Regression test for a P1 review finding on ADR-0003 Phase 3: a peer
/// proposing Bluetooth to a device that has Bluetooth disabled locally
/// can't simply be told "no" via silence -- if this device has no
/// preference of its own recorded, `adopt_peer_transport_preference`'s
/// `Ineligible` rejection previously had no stored preference to offer
/// back either, so the sender never learned to fall back and a live
/// Network session got force-closed for nothing. Proves the fix: an
/// explicit Network counter-proposal is sent even with no stored
/// preference, and the session that received the (rejected) proposal is
/// *not* force-closed as a side effect of the rejection.
#[tokio::test(flavor = "multi_thread")]
async fn ineligible_switch_transport_produces_a_network_fallback_instead_of_silence() {
    let (server, server_db) = server_state("transport-ineligible-switch-fallback");
    seed_paired_device(&server_db, "peer-client");
    // Bluetooth left at test_conn-equivalent default (disabled) for this
    // pair -- the condition under test.

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;

    send_frame(
        link.as_mut(),
        &PeerFrame::SwitchTransport {
            to: TransportKind::Bluetooth,
            requested_at: "2026-04-07T00:00:01Z".to_string(),
        },
    )
    .await
    .expect("send the (unsupportable) Bluetooth proposal");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for the Network fallback");
        match recv_frame(link.as_mut()).await {
            Some(Ok(PeerFrame::SwitchTransport { to, .. })) => {
                assert_eq!(to, TransportKind::TcpWs, "must fall back to Network, not stay silent");
                break;
            }
            Some(Ok(PeerFrame::BluetoothAddressUpdate { .. })) => continue,
            other => panic!("expected a Network fallback SwitchTransport, got {other:?}"),
        }
    }

    sleep(Duration::from_millis(50)).await;
    assert!(
        server.has_session("peer-client"),
        "rejecting an unsupportable proposal must not force-close the live session"
    );
}

/// Regression test for a P1 review finding on ADR-0003 Phase 3: disabling
/// Bluetooth for a pair that was pinned to it must not just clear this
/// device's own row -- if the peer is the sole deterministic dialer and
/// still holds the old Bluetooth pin, it never learns to fall back
/// (Network dial stands down because of its own pin; Bluetooth is now
/// rejected by this device). Proves
/// `device_connection_set_bluetooth_transport_with_state_impl` relays the
/// redirect to a live peer, not just the local DB.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_bluetooth_notifies_a_live_peer_to_fall_back_to_network() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-disable-bluetooth-notifies");
    seed_paired_device(&server_db, "peer-client");

    let port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = tcp_ws::dial("127.0.0.1".parse().unwrap(), port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");
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

    // Drain the notification this setup step itself sends (pinning to
    // Bluetooth), so the assertions below only see the disable's own
    // redirect notification. Skips past a real machine's own Bluetooth
    // self-report too, same as the other tests in this file.
    loop {
        match recv_frame(link.as_mut()).await {
            Some(Ok(PeerFrame::SwitchTransport { to: TransportKind::Bluetooth, .. })) => break,
            Some(Ok(PeerFrame::BluetoothAddressUpdate { .. })) => continue,
            other => panic!("expected the setup's own Bluetooth pin notification, got {other:?}"),
        }
    }

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

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for the Network redirect");
        match recv_frame(link.as_mut()).await {
            Some(Ok(PeerFrame::SwitchTransport { to, .. })) => {
                assert_eq!(to, TransportKind::TcpWs);
                break;
            }
            Some(Ok(PeerFrame::BluetoothAddressUpdate { .. })) => continue,
            other => panic!("expected a Network redirect SwitchTransport, got {other:?}"),
        }
    }
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

    match tokio::time::timeout(Duration::from_millis(500), recv_frame(link.as_mut())).await {
        Ok(Some(Ok(PeerFrame::BluetoothAddressUpdate { address }))) => {
            assert_eq!(address, "11:22:33:44:55:66");
        }
        other => panic!("expected a refreshed BluetoothAddressUpdate after the address changed, got {other:?}"),
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
    use crate::services::transport::selection::TRANSPORT_UNRESPONSIVE_THRESHOLD;

    let (state, _db) = server_state("transport-network-effectively-available");

    // No presence at all: never effectively available regardless of failures
    // (this test can't inject synthetic presence — that's owned by the
    // discovery worker's internal state — so it verifies the failure-count
    // threshold mechanics directly; presence itself is exercised end-to-end
    // by the existing discovery/pairing tests).
    assert!(!state.network_effectively_available("peer-x"));

    for _ in 0..(TRANSPORT_UNRESPONSIVE_THRESHOLD - 1) {
        state.record_tcp_dial_failure("peer-x");
    }
    assert_eq!(
        state.tcp_dial_failure_count("peer-x"),
        TRANSPORT_UNRESPONSIVE_THRESHOLD - 1,
        "below threshold yet"
    );

    state.record_tcp_dial_failure("peer-x");
    assert_eq!(
        state.tcp_dial_failure_count("peer-x"),
        TRANSPORT_UNRESPONSIVE_THRESHOLD,
        "at threshold"
    );

    // A later success resets the counter (transient blip, not permanent).
    state.record_tcp_dial_success("peer-x");
    assert_eq!(state.tcp_dial_failure_count("peer-x"), 0);
}

/// Bluetooth's counterpart to the test above (ADR-0003 Phase 2): a fresh
/// peer with zero attempts is reliable by default -- "no reason to distrust
/// it" is the bar, not "has been proven to work" -- then demotes once
/// consecutive failures reach the shared threshold, and a later success
/// resets it.
#[test]
fn bluetooth_effectively_reliable_demotes_after_repeated_failures_and_resets_on_success() {
    use crate::services::transport::selection::TRANSPORT_UNRESPONSIVE_THRESHOLD;

    let (state, _db) = server_state("transport-bluetooth-effectively-reliable");

    assert!(
        state.bluetooth_effectively_reliable("peer-x"),
        "a never-attempted peer must default to reliable"
    );

    for _ in 0..(TRANSPORT_UNRESPONSIVE_THRESHOLD - 1) {
        state.record_bluetooth_dial_failure("peer-x");
    }
    assert!(
        state.bluetooth_effectively_reliable("peer-x"),
        "below threshold yet"
    );

    state.record_bluetooth_dial_failure("peer-x");
    assert_eq!(
        state.bluetooth_dial_failure_count("peer-x"),
        TRANSPORT_UNRESPONSIVE_THRESHOLD
    );
    assert!(
        !state.bluetooth_effectively_reliable("peer-x"),
        "at threshold, no longer reliable"
    );

    state.record_bluetooth_dial_success("peer-x");
    assert_eq!(state.bluetooth_dial_failure_count("peer-x"), 0);
    assert!(state.bluetooth_effectively_reliable("peer-x"));
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
            let _ = send_frame(link.as_mut(), &PeerFrame::AuthOk { protocol_version: 1 }).await;
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
