//! End-to-end proof that the transport abstraction works: two independent
//! adapters (`tcp_ws`, `sim`) carry the exact same `crate::services::communication::pairing::run_peer_gate`/
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
use crate::services::communication::pairing::{channels, DeviceConnectionState};
use crate::services::communication::sync::session;
use crate::services::communication::sync::types::PeerFrame;
use crate::services::communication::channel::{recv_frame, send_frame, tcp_ws, DataLink, Transport, ChannelKind};

/// A Bluetooth-kind link for tests, carried over a plain TCP socket.
///
/// Production has no such thing any more, deliberately: a stand-in radio
/// that shipped, appeared in the channel list and could be reached by
/// setting an environment variable was worse than the gap it filled --
/// `ble-gatt`'s mock broker covers that ground by faking the radio
/// underneath `ble`, leaving the whole Bluetooth path above it real.
///
/// Tests still need a second channel they can drive without a radio, and
/// that need is a test's own. So the stand-in lives here, where nothing
/// ships it, and it carries the framing that went with it.
mod bluetooth_stub {
    use async_trait::async_trait;
    use tokio::net::{TcpListener, TcpStream};

    use crate::services::communication::channel::{DataLink, Transport, BoxDialFuture, ChannelKind};
    use crate::services::communication::pairing::DeviceConnectionState;
    use std::path::PathBuf;

    pub mod length_delimited {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

        pub async fn write<W: tokio::io::AsyncWrite + Unpin>(
            writer: &mut W,
            payload: &[u8],
        ) -> Result<(), String> {
            let len = u32::try_from(payload.len()).map_err(|_| "frame too large".to_string())?;
            writer
                .write_all(&len.to_be_bytes())
                .await
                .map_err(|err| format!("write frame length: {err}"))?;
            writer
                .write_all(payload)
                .await
                .map_err(|err| format!("write frame payload: {err}"))?;
            writer
                .flush()
                .await
                .map_err(|err| format!("flush frame: {err}"))
        }

        /// A reader that survives being cancelled mid-frame.
        ///
        /// `read` below is not cancellation-safe, and the session loop reads
        /// inside a `tokio::select!` -- so every ping it has to send and every
        /// outbox event it has to forward drops the in-flight read. Dropped
        /// after the four length bytes and before the payload, those four bytes
        /// are simply gone: the next read then takes the payload's first four
        /// bytes as a length. For our frames that is `{"v`, or 2065856034,
        /// which fails the size check and kills an authenticated session.
        ///
        /// This keeps whatever has arrived in a buffer that belongs to the
        /// link rather than to the future, and fills it with `read_buf`, which
        /// *is* cancellation-safe: if the future is dropped, nothing was taken
        /// from the socket that is not already in the buffer. Cancelling costs
        /// a wasted poll and nothing else.
        #[derive(Default)]
        pub struct FrameReader {
            pending: Vec<u8>,
        }

        impl FrameReader {
            /// One frame, or `None` at a clean EOF.
            pub async fn read<R: tokio::io::AsyncRead + Unpin>(
                &mut self,
                reader: &mut R,
            ) -> Option<Result<Option<Vec<u8>>, String>> {
                loop {
                    match self.take_frame() {
                        Some(Ok(frame)) => return Some(Ok(Some(frame))),
                        Some(Err(err)) => return Some(Err(err)),
                        None => {}
                    }
                    match reader.read_buf(&mut self.pending).await {
                        Ok(0) => {
                            return if self.pending.is_empty() {
                                Some(Ok(None))
                            } else {
                                // A frame was promised and the socket closed
                                // inside it. Saying EOF here would report a
                                // clean shutdown for a truncated one.
                                Some(Err(format!(
                                    "connection closed mid-frame with {} bytes buffered",
                                    self.pending.len()
                                )))
                            }
                        }
                        Ok(_) => continue,
                        Err(err) => return Some(Err(format!("read frame: {err}"))),
                    }
                }
            }

            fn take_frame(&mut self) -> Option<Result<Vec<u8>, String>> {
                if self.pending.len() < 4 {
                    return None;
                }
                let len = u32::from_be_bytes([
                    self.pending[0],
                    self.pending[1],
                    self.pending[2],
                    self.pending[3],
                ]);
                if len > MAX_FRAME_LEN {
                    return Some(Err(format!(
                        "frame length {len} exceeds max {MAX_FRAME_LEN}; first bytes were {:?}",
                        String::from_utf8_lossy(&self.pending[..4])
                    )));
                }
                let total = 4 + len as usize;
                if self.pending.len() < total {
                    return None;
                }
                let frame = self.pending[4..total].to_vec();
                self.pending.drain(..total);
                Some(Ok(frame))
            }
        }

        /// `Ok(None)` means clean EOF (peer closed the connection).
        pub async fn read<R: tokio::io::AsyncRead + Unpin>(
            reader: &mut R,
        ) -> Result<Option<Vec<u8>>, String> {
            let mut len_buf = [0_u8; 4];
            match reader.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(err) => return Err(format!("read frame length: {err}")),
            }
            let len = u32::from_be_bytes(len_buf);
            if len > MAX_FRAME_LEN {
                // Show the bytes, not just the number they decoded to. A length
                // this wrong means the stream is not carrying length-prefixed
                // frames at all, and what it *is* carrying names the writer --
                // "{\"ty" reads very differently from an HTTP verb.
                return Err(format!(
                    "frame length {len} exceeds max {MAX_FRAME_LEN}; first bytes were {:?}",
                    String::from_utf8_lossy(&len_buf)
                ));
            }
            let mut payload = vec![0_u8; len as usize];
            reader
                .read_exact(&mut payload)
                .await
                .map_err(|err| format!("read frame payload: {err}"))?;
            Ok(Some(payload))
        }
    }


    pub struct StubDataLink {
        stream: TcpStream,
        reader: length_delimited::FrameReader,
    }

    impl StubDataLink {
        pub fn new(stream: TcpStream) -> Self {
            Self { stream, reader: length_delimited::FrameReader::default() }
        }
    }

    #[async_trait]
    impl DataLink for StubDataLink {
        fn kind(&self) -> ChannelKind {
            ChannelKind::Bluetooth
        }

        async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
            length_delimited::write(&mut self.stream, &payload).await
        }

        async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
            match self.reader.read(&mut self.stream).await? {
                Ok(Some(payload)) => Some(Ok(payload)),
                Ok(None) => None,
                Err(err) => Some(Err(err)),
            }
        }

        fn peer_addr(&self) -> Option<String> {
            self.stream.peer_addr().ok().map(|addr| addr.ip().to_string())
        }
    }

    pub async fn dial(port: u16) -> Result<Box<dyn DataLink>, String> {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|err| format!("stub connect 127.0.0.1:{port} failed: {err}"))?;
        Ok(Box::new(StubDataLink::new(stream)))
    }

    pub struct StubTransport;

    #[async_trait]
    impl Transport for StubTransport {
        fn kind(&self) -> ChannelKind {
            ChannelKind::Bluetooth
        }

        fn dial(&self, _peer_device_id: &str, _addr: &str, port: u16) -> BoxDialFuture {
            Box::pin(async move { dial(port).await })
        }
    }

    pub async fn run_server(state: DeviceConnectionState, db_path: PathBuf, port: u16) {
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(err) => panic!("stub server failed to bind :{port}: {err}"),
        };
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let link: Box<dyn DataLink> = Box::new(StubDataLink::new(stream));
                    let state = state.clone();
                    let db_path = db_path.clone();
                    tokio::spawn(crate::services::communication::pairing::run_peer_gate(
                        link, state, db_path,
                    ));
                }
                Err(err) => panic!("stub server accept failed: {err}"),
            }
        }
    }
}


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
    // A pair is only reachable over channels it has configured, and the
    // session gate fails closed on the rest. Every real network pairing
    // configures this, so a seeded pair that skipped it would be rejected at
    // `Auth` -- not a bug these tests are about.
    channels::configure(&mut conn, peer_device_id, ChannelKind::Network, true, None)
        .expect("set the pair's Network channel up");
}

/// Sets this pair's Bluetooth channel up and switches it on, with a stored
/// address. Callers that also need the peer to look OS-bonded arrange that
/// separately via the `FINI_BLUETOOTH_PAIRED_ADDRESSES` escape hatch
/// (holding `BLUETOOTH_ADDRESS_ENV_LOCK`) for the address used here.
fn seed_bluetooth_enabled_peer(db_path: &PathBuf, peer_device_id: &str, address: &str) {
    let mut conn = open_db_at_path(db_path);
    crate::services::communication::pairing::channels::configure(
        &mut conn,
        peer_device_id,
        ChannelKind::Bluetooth,
        true,
        Some(address),
    )
    .expect("set the pair's Bluetooth channel up");
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
/// `pairing::commands::tests` (not a separate lock of the same
/// name -- an earlier version of this comment claimed a disjoint set of
/// tests justified two locks, but both modules set/clear the exact same
/// env var, so two locks could still race with *each other*, observed as
/// intermittent failures once enough tests in both files touched it).
use crate::services::communication::pairing::BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK as BLUETOOTH_ADDRESS_ENV_LOCK;

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
        Some(ChannelKind::Network)
    );
}

/// ADR-0003 revision: pinning a peer to a transport different from the one
/// currently primary just flips which already-connected transport is
/// primary -- no wire frame, no session disturbed on either transport,
/// since both stay connected regardless of the pin.
#[tokio::test(flavor = "multi_thread")]
async fn set_preferred_channel_flips_primary_without_disturbing_either_session() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("transport-set-preferred-flips-primary");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    let tcp_port = free_port().await;
    let loopback_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), tcp_port));
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), loopback_port));
    sleep(Duration::from_millis(100)).await;

    let mut tcp_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port).await.expect("dial tcp");
    session::perform_client_auth(tcp_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("tcp auth should succeed for paired device");
    let mut loopback_link = bluetooth_stub::dial(loopback_port).await.expect("dial loopback");
    session::perform_client_auth(loopback_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("sim (bluetooth-kind) auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.primary_transport("peer-client"), Some(ChannelKind::Network));

    let mut conn = open_db_at_path(&server_db);
    let updated = crate::services::communication::pairing::device_connection_set_primary_channel_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(ChannelKind::Bluetooth),
    )
    .expect("choose Bluetooth as the primary channel");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    let bluetooth_row = updated
        .iter()
        .find(|status| status.kind == ChannelKind::Bluetooth)
        .expect("a Bluetooth row");
    assert!(bluetooth_row.primary, "the choice must come back on the row");

    // The server-side session actually claims under the real wire kind
    // (`Sim`, standing in for Bluetooth here) -- `AsBluetooth` only affects
    // what the *client* reports, not what `run_peer_gate`'s accept side
    // sees from its own `link.kind()`. `ChannelKind::from(ChannelKind)`
    // collapses Sim/Bluetooth/LoRa to the same Bluetooth channel either way.
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(ChannelKind::Bluetooth),
        "primary must flip immediately, without waiting for a reconnect"
    );
    assert!(
        server.has_session_on("peer-client", ChannelKind::Network),
        "the Network session must stay connected -- only primary-ness changed"
    );
    assert!(
        server.has_session_on("peer-client", ChannelKind::Bluetooth),
        "the bluetooth-role session must stay connected"
    );
}

/// Regression test for a P1 review finding: a stale "configured" row
/// (`DeviceView`'s polling only refreshes session liveness, not full
/// eligibility -- see the frontend's own `refreshLiveConnectedState` doc
/// comment) can stay clickable well after the channel it names has been
/// switched off. `device_connection_set_primary_channel_impl` must
/// re-validate that itself rather than trusting the click -- storing a
/// choice that can never actually carry traffic just relocates the
/// stranding hazard instead of preventing it.
#[tokio::test(flavor = "multi_thread")]
async fn set_primary_channel_refuses_a_bluetooth_pin_when_not_currently_eligible() {
    let (server, server_db) = server_state("channel-set-primary-bluetooth-ineligible");
    seed_paired_device(&server_db, "peer-client");
    // No Bluetooth channel configured at all -- the condition under test.

    let mut conn = open_db_at_path(&server_db);
    let err = crate::services::communication::pairing::device_connection_set_primary_channel_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(ChannelKind::Bluetooth),
    )
    .expect_err("must refuse a channel that is not switched on");
    assert!(err.contains("Switch the channel on"), "got: {err}");

    assert_eq!(
        channels::primary_kind(&mut conn, "peer-client"),
        None,
        "a refused choice must not be persisted"
    );
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
    // Written straight to the table, bypassing the command's own "switch it
    // on first" check: the point here is the dial path, not the choice.
    channels::configure(
        &mut dialer_conn,
        &responder.identity.device_id,
        ChannelKind::Bluetooth,
        true,
        None,
    )
    .expect("set the Bluetooth channel up");
    channels::set_primary(
        &mut dialer_conn,
        &responder.identity.device_id,
        Some(ChannelKind::Bluetooth),
    )
    .expect("choose Bluetooth as primary");

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
        if dialer.has_session_on(&responder.identity.device_id, ChannelKind::Network) {
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
        !dialer.has_session_on(&responder.identity.device_id, ChannelKind::Network),
        "must not have established anything against the dead port"
    );

    let real_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(responder.clone(), responder_db.clone(), real_port));
    sleep(Duration::from_millis(100)).await;
    dialer.note_presence_for_test(&responder.identity.device_id, "127.0.0.1", real_port);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut established = false;
    while tokio::time::Instant::now() < deadline {
        if dialer.has_session_on(&responder.identity.device_id, ChannelKind::Network) {
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

/// Regression test for a P1 review finding: switching a channel off for a
/// pair whose traffic was pinned to it must not leave the pair pointed at a
/// channel that can never reconnect. Turning it off releases the choice
/// (ADR-0007), and selection falls straight back to the already-connected
/// Network session -- no wire notification needed, since both channels stay
/// connected regardless of the choice and there is nothing to relay.
#[tokio::test(flavor = "multi_thread")]
async fn switching_bluetooth_off_releases_the_primary_and_flips_it_to_network() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("channel-switch-bluetooth-off-releases-primary");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");

    let tcp_port = free_port().await;
    let loopback_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(server.clone(), server_db.clone(), tcp_port));
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), loopback_port));
    sleep(Duration::from_millis(100)).await;

    let mut tcp_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port).await.expect("dial tcp");
    session::perform_client_auth(tcp_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("tcp auth should succeed for paired device");
    let mut loopback_link = bluetooth_stub::dial(loopback_port).await.expect("dial loopback");
    session::perform_client_auth(loopback_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("sim (bluetooth-kind) auth should succeed for paired device");
    sleep(Duration::from_millis(50)).await;

    let mut conn = open_db_at_path(&server_db);
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    crate::services::communication::pairing::device_connection_set_primary_channel_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        Some(ChannelKind::Bluetooth),
    )
    .expect("choose Bluetooth as primary");
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
    // See the sibling test above for why this is `Sim`, not `Bluetooth`.
    assert_eq!(server.primary_transport("peer-client"), Some(ChannelKind::Bluetooth));

    crate::services::communication::pairing::device_connection_set_channel_enabled_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
        false,
    )
    .expect("switch bluetooth off");
    assert_eq!(
        channels::primary_kind(&mut conn, "peer-client"),
        None,
        "switching a channel off must release the primary rather than leave it pointed at nothing"
    );
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(ChannelKind::Network),
        "primary must flip to the already-connected Network session immediately"
    );
}

/// Regression test for a second P1 review finding on this PR: disabling
/// Bluetooth while a `ChannelKind::Bluetooth` session is already live
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
/// kind, not the loopback radio, which aren't governed by that column at all
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
        channels::configure(
            &mut conn,
            "peer-client",
            ChannelKind::Bluetooth,
            true,
            Some("127.0.0.1"),
        )
        .expect("switch bluetooth on for the seeded peer");
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
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_server, gate_db).await;
    });
    sleep(Duration::from_millis(100)).await;

    let mut tcp_link = tcp_ws::dial("127.0.0.1".parse().unwrap(), tcp_port).await.expect("dial tcp");
    session::perform_client_auth(tcp_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("tcp auth should succeed for paired device");

    let ble_stream = TcpStream::connect(("127.0.0.1", ble_port)).await.unwrap();
    let mut ble_link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(ble_stream));
    session::perform_client_auth(ble_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("bluetooth-kind auth should succeed for a bonded, enabled paired device");
    sleep(Duration::from_millis(50)).await;

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    // Network is primary by default (network-first) -- the common
    // real-world case for this finding: disabling Bluetooth from Settings
    // while a healthy Network session is already carrying traffic, not the
    // pinned case the sibling test above covers.
    assert_eq!(server.primary_transport("peer-client"), Some(ChannelKind::Network));

    let mut conn = open_db_at_path(&server_db);
    crate::services::communication::pairing::device_connection_set_channel_enabled_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
        false,
    )
    .expect("disable bluetooth");

    // Network drops immediately after -- no sleep, so the Bluetooth
    // session's own `run_session` loop has not necessarily processed the
    // `Close` command `close_session_on` just sent it. Without
    // `recompute_primary_locked`'s `bluetooth_enabled` exclusion, this
    // falls back to the still-technically-connected Bluetooth session.
    server.release_session("peer-client", ChannelKind::Network, &server_db);

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
        !server.has_session_on("peer-client", ChannelKind::Bluetooth),
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
/// unpinned scenario `device_connection_set_bluetooth_channel_with_
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
        channels::configure(
            &mut conn,
            "peer-client",
            ChannelKind::Bluetooth,
            true,
            Some("127.0.0.1"),
        )
        .expect("switch bluetooth on for the seeded peer");
    }

    let ble_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ble_port = ble_listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = ble_listener.accept().await else {
            return;
        };
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_server, gate_db).await;
    });
    sleep(Duration::from_millis(100)).await;

    let ble_stream = TcpStream::connect(("127.0.0.1", ble_port)).await.unwrap();
    let mut ble_link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(ble_stream));
    session::perform_client_auth(ble_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("bluetooth-kind auth should succeed for a bonded, enabled paired device");
    sleep(Duration::from_millis(50)).await;
    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");

    assert_eq!(
        server.primary_transport("peer-client"),
        Some(ChannelKind::Bluetooth),
        "bluetooth should be primary -- it's the only connected transport, and unpinned"
    );

    let mut conn = open_db_at_path(&server_db);
    crate::services::communication::pairing::device_connection_set_channel_enabled_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
        false,
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
            if matches!(command, crate::services::communication::sync::types::SessionCommand::Close) {
                consumer_server.release_session("peer-client", ChannelKind::Bluetooth, &consumer_db);
                break;
            }
        }
    });

    assert!(server.try_claim_session("peer-client", ChannelKind::Bluetooth, tx, &server_db));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut closed = false;
    while tokio::time::Instant::now() < deadline {
        if !server.has_session_on("peer-client", ChannelKind::Bluetooth) {
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
    assert!(server.try_claim_session("peer-client", ChannelKind::Network, tx, &server_db));
    assert!(server.has_session_on("peer-client", ChannelKind::Network));

    let doomed_db_path = std::path::PathBuf::from("/nonexistent-directory-for-fini-tests/fini.db");
    let peer = "peer-client".to_string();
    let server_for_task = server.clone();
    let handle = tokio::spawn(async move {
        server_for_task.release_session(&peer, ChannelKind::Network, &doomed_db_path);
    });

    sleep(Duration::from_millis(20)).await;
    assert!(
        !server.has_session_on("peer-client", ChannelKind::Network),
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
        channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, true, None)
            .expect("switch bluetooth on for the seeded peer");
    }

    let (tcp_tx, _tcp_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", ChannelKind::Network, tcp_tx, &server_db));
    let (ble_tx, _ble_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", ChannelKind::Bluetooth, ble_tx, &server_db));

    assert_eq!(
        server.primary_transport("peer-client"),
        Some(ChannelKind::Network),
        "network wins by default with both connected"
    );

    let doomed_db_path = std::path::PathBuf::from("/nonexistent-directory-for-fini-tests/fini.db");
    let peer = "peer-client".to_string();
    let server_for_task = server.clone();
    let handle = tokio::spawn(async move {
        server_for_task.release_session(&peer, ChannelKind::Network, &doomed_db_path);
    });

    sleep(Duration::from_millis(20)).await;
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(ChannelKind::Bluetooth),
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
        channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, true, None)
            .expect("switch bluetooth on for the seeded peer");
    }

    let (tcp_tx, _tcp_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", ChannelKind::Network, tcp_tx, &server_db));
    let (ble_tx, _ble_rx) = tokio::sync::mpsc::channel(4);
    assert!(server.try_claim_session("peer-client", ChannelKind::Bluetooth, ble_tx, &server_db));
    assert_eq!(server.primary_transport("peer-client"), Some(ChannelKind::Network));

    let mut conn = open_db_at_path(&server_db);
    crate::services::communication::pairing::device_connection_set_channel_enabled_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
        false,
    )
    .expect("disable bluetooth");
    assert!(
        server.has_session_on("peer-client", ChannelKind::Bluetooth),
        "the Bluetooth session must still be present -- its async Close hasn't been processed"
    );

    let doomed_db_path = std::path::PathBuf::from("/nonexistent-directory-for-fini-tests/fini.db");
    let peer = "peer-client".to_string();
    let server_for_task = server.clone();
    let handle = tokio::spawn(async move {
        server_for_task.release_session(&peer, ChannelKind::Network, &doomed_db_path);
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
    let envelope = crate::services::communication::channel::envelope::FrameEnvelope::new(
        crate::services::communication::channel::envelope::EncScheme::None,
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

/// ADR-0006: a self-report records the address and never touches the
/// switch -- **not even when the reported address is OS-bonded**, which is
/// what this test pins.
///
/// The bond used to decide this, and that made a background message able to
/// flip a channel on or off. Off was the damaging direction: during
/// hardware verification the peer reported its address, this side found no
/// bond, and switched a working pair off. Since the bond is now consulted
/// nowhere on the dial path, the honest rule is that a self-report carries
/// no authority over the switch in either direction.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_does_not_switch_a_channel_on_even_for_a_bonded_address() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "AA:BB:CC:DD:EE:FF");

    let (server, server_db) = server_state("channel-tcpws-self-report-enable");
    seed_paired_device(&server_db, "peer-client");
    {
        // The channel has to exist for an address to be recorded against
        // it at all -- a self-report must never be what sets a channel up.
        // Switched on, so this test isolates the one thing it is about: a
        // bonded address arriving over the wire does not move the switch.
        let mut conn = open_db_at_path(&server_db);
        channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, true, None)
            .expect("set the Bluetooth channel up");
    }
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
    let row = channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth)
        .expect("the Bluetooth channel row");
    assert_eq!(row.address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    assert!(
        !row.is_primary,
        "a self-report must not choose the channel either -- that is the \
         person's call, made on the row"
    );

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
}

/// Mirror of the above for a pair that has no Bluetooth channel at all: the
/// self-report is dropped entirely.
///
/// A row existing is what "configured" means, so recording the address would
/// mean a background message from the peer setting a channel up on this
/// device -- the page would offer a Bluetooth row the person never asked
/// for. There is also nothing for the address to be useful to: nothing dials
/// it (ADR-0006), it is diagnostics for a channel that exists.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_self_report_does_not_set_up_a_channel_that_was_never_configured() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();

    let (server, server_db) = server_state("channel-tcpws-self-report-no-channel");
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
    assert!(
        channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).is_none(),
        "a self-report must not be what sets a channel up"
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
async fn loopback_gate_accepts_a_paired_device_and_claims_the_bluetooth_session() {
    let (server, server_db) = server_state("channel-loopback-accept");
    seed_paired_device(&server_db, "peer-client");
    // The loopback radio *is* the Bluetooth channel where there is no
    // hardware, so the pair needs that channel set up for the gate to let it
    // in -- exactly as a real radio would.
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    let port = free_port().await;
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), port));
    sleep(Duration::from_millis(100)).await;

    let mut link = bluetooth_stub::dial(port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");

    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.primary_transport("peer-client"), Some(ChannelKind::Bluetooth));
}

/// ADR-0003 revision's core new guarantee: both Network and Bluetooth can
/// be simultaneously connected and claimed for the same peer -- Network
/// becomes primary (it wins whenever connected), but the Sim (playing
/// Bluetooth's role) session is not rejected or torn down; it stays live,
/// connected but not primary. This is what makes green a per-transport,
/// continuously-reproven property rather than something borrowed from
/// whichever session happens to be "the" one.
#[tokio::test(flavor = "multi_thread")]
async fn both_channels_can_be_simultaneously_connected_for_the_same_peer() {
    let (server, server_db) = server_state("channel-dual-connect");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_enabled_peer(&server_db, "peer-client", "AA:BB:CC:DD:EE:FF");
    let tcp_port = free_port().await;
    let loopback_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        tcp_port,
    ));
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), loopback_port));
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
        Some(ChannelKind::Network)
    );

    // A second connection on a *different* transport must be accepted, not
    // rejected -- the old sticky single-session invariant no longer holds.
    let mut second_link = bluetooth_stub::dial(loopback_port).await.expect("dial loopback");
    session::perform_client_auth(second_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("a session on a second transport must also be accepted");
    sleep(Duration::from_millis(50)).await;

    assert!(server.has_session_on("peer-client", ChannelKind::Network));
    assert!(server.has_session_on("peer-client", ChannelKind::Bluetooth));
    assert_eq!(
        server.primary_transport("peer-client"),
        Some(ChannelKind::Network),
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
    let loopback_port = free_port().await;
    tokio::spawn(tcp_ws::run_server_on_port(
        server.clone(),
        server_db.clone(),
        tcp_port,
    ));
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), loopback_port));
    sleep(Duration::from_millis(100)).await;

    let adapters: Vec<(Box<dyn Transport>, u16, ChannelKind)> = vec![
        (Box::new(tcp_ws::TcpWsTransport), tcp_port, ChannelKind::Network),
        (Box::new(bluetooth_stub::StubTransport), loopback_port, ChannelKind::Bluetooth),
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

/// `pairing::commands::send_pair_ws` is a one-shot sender
/// independent of `TcpWsDataLink` (connect, send one frame, close) — it does
/// not go through `DataLink::send`, so nothing structurally forces it to stay
/// wire-compatible with what `run_peer_gate`/`codec::decode_frame` expect
/// on the receiving end. This regression-tests that compatibility directly:
/// a real `PairRequest` sent via the production
/// `device_connection_send_pair_request_impl` path must be readable by a
/// real `tcp_ws` listener and land in the receiver's incoming-request queue.
#[tokio::test(flavor = "multi_thread")]
async fn send_pair_request_is_readable_by_the_receiving_gate() {
    use crate::services::communication::pairing::types::DevicePairRequestInput;
    use crate::services::communication::pairing::{
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
    use crate::services::communication::pairing::types::{DevicePairRequestAckInput, DevicePairRequestInput};
    use crate::services::communication::pairing::{
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
/// two-sided session, it becomes green (`channel_reliable`). Uses a short
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
        !server.channel_reliable("peer-client", ChannelKind::Network),
        "a freshly claimed session must not already be green"
    );
    // Regression test for a P1 review finding on this PR: the lightweight
    // live-poll surface (`device_connection_channel_liveness`) must
    // reflect this amber-not-green state, not leave it frozen -- the whole
    // point of it existing is to let a Bluetooth-only peer's row (never
    // covered by the network-presence-gated full poll) stay current
    // without that poll's cost.
    let liveness_before = crate::services::communication::pairing::device_connection_channel_liveness_impl(
        &server,
        "peer-client".to_string(),
    );
    let network_liveness_before = liveness_before
        .iter()
        .find(|l| l.kind == crate::services::communication::pairing::ChannelKind::Network)
        .expect("network liveness row");
    assert!(network_liveness_before.connected);
    assert!(
        network_liveness_before.reason.is_some(),
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
        server.channel_reliable("peer-client", ChannelKind::Network),
        "green once both directions of the ping/ack proof are complete"
    );

    let liveness_after = crate::services::communication::pairing::device_connection_channel_liveness_impl(
        &server,
        "peer-client".to_string(),
    );
    let network_liveness_after = liveness_after
        .iter()
        .find(|l| l.kind == crate::services::communication::pairing::ChannelKind::Network)
        .expect("network liveness row");
    assert!(
        network_liveness_after.reason.is_none(),
        "the lightweight live-poll surface must also report green (code: None) once the \
         ping/ack proof completes, not stay frozen at the pre-proof amber snapshot"
    );
}

// The mutual-dial race that `loopback::should_dial_fallback_peer`'s deterministic
// dialer rule fixes is unit-tested directly there, mirroring
// `tcp_ws::should_dial_peer`'s own test — reproducing the actual network
// race end-to-end in an integration test proved unreliable (the exact
// collision needs both connects landing in the same async poll step, and a
// sleep-driven loopback test can't force that deterministically) without
// adding disproportionate complexity for what a pure function test already
// covers exactly.

/// Wraps a real link but reports a different `ChannelKind` — lets these
/// tests drive `run_peer_gate`'s Bluetooth-specific enablement check using
/// `sim`'s real, already-proven TCP+length-delimited wire protocol, without
/// needing an actual BLE stack (no adapter's `DataLink` impl is swappable at the
/// `kind()` level otherwise).
struct AsBluetooth(Box<dyn DataLink>);

#[async_trait]
impl DataLink for AsBluetooth {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
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
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_server, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("a paired but bluetooth-disabled device must be rejected over a Bluetooth-kind link");
    assert!(err.contains("bluetooth disabled"), "unexpected error: {err}");
}

/// Mirror of the above with Bluetooth explicitly enabled for the pair: the
/// same link kind must now authenticate and claim the session as
/// `ChannelKind::Bluetooth`, proving the new check only rejects the
/// disabled case rather than breaking Bluetooth accepts outright.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_gate_accepts_paired_device_with_bluetooth_enabled() {
    // The stored address and the OS-paired allow-list below are no longer
    // required to pass: ADR-0006 deleted `check_bluetooth_bond`, which used
    // to demand that the connecting link's `peer_addr()` match the pair's
    // stored `bluetooth_address` and that the address be OS-paired. They are
    // left in place because this test's subject is the *enabled* gate, and
    // keeping the surrounding setup unchanged keeps it a true mirror of the
    // disabled-case test above.
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "127.0.0.1");

    let (server, server_db) = server_state("transport-ble-gate-enabled");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        channels::configure(
            &mut conn,
            "peer-client",
            ChannelKind::Bluetooth,
            true,
            Some("127.0.0.1"),
        )
        .expect("switch bluetooth on for the seeded peer");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_server, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("a bluetooth-enabled, bonded paired device should authenticate over a Bluetooth-kind link");

    sleep(Duration::from_millis(50)).await;
    assert_eq!(server.primary_transport("peer-client"), Some(ChannelKind::Bluetooth));

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
}

/// Regression test for the second gap Codex flagged on PR #140's re-review:
/// ADR-0006 reversed this case, and it is kept rather than deleted because
/// the reversal is the whole point of that decision.
///
/// A Bluetooth peer whose connecting address matches nothing stored, and
/// which is not OS-bonded, used to be rejected. It is now **accepted**: the
/// address a peer connects from is meaningless when Android rotates it, and
/// identity comes from the `Auth` frame, which
/// `specs/device-connect/README.md` already names as the trust boundary.
/// Rejecting this case is exactly what made the transport unable to connect
/// at all.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_gate_accepts_a_peer_whose_address_matches_nothing_stored() {
    let _guard = BLUETOOTH_ADDRESS_ENV_LOCK.lock().unwrap();
    // An allow-list that matches neither the stored address nor the
    // connecting one, so nothing in this test is OS-bonded.
    std::env::set_var("FINI_BLUETOOTH_PAIRED_ADDRESSES", "ZZ:ZZ:ZZ:ZZ:ZZ:ZZ");

    let (server, server_db) = server_state("transport-ble-gate-address-mismatch");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        channels::configure(
            &mut conn,
            "peer-client",
            ChannelKind::Bluetooth,
            true,
            Some("AA:BB:CC:DD:EE:FF"),
        )
        .expect("switch bluetooth on for the seeded peer");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_server = server.clone();
    let gate_db = server_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        // Connects as "127.0.0.1" (LoopbackDataLink's real peer_addr), not the
        // Connects as "127.0.0.1" (LoopbackDataLink's real peer_addr), which is
        // neither the stored address nor OS-bonded -- the shape of every
        // real Android peer, which advertises under a rotating address that
        // by construction matches nothing stored.
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_server, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("an authenticated peer must be accepted regardless of its address");

    std::env::remove_var("FINI_BLUETOOTH_PAIRED_ADDRESSES");
}

/// ADR 0002 Phase 3: a `PairRequest` delivered over a Bluetooth-kind link
/// must be flagged `via_bluetooth`, with `from_bluetooth_address` set to the
/// address actually *observed* on that connection (`DataLink::peer_addr()`) --
/// trusted over any self-report, since the sender has no network endpoint
/// fields to self-report through this transport in the first place.
#[tokio::test(flavor = "multi_thread")]
async fn pair_request_over_a_bluetooth_link_captures_the_observed_address() {
    use crate::services::communication::pairing::types::PairRequestPayload;
    use crate::services::communication::pairing::{
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
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
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
    use crate::services::communication::pairing::types::PairCompletePayload;
    use crate::services::communication::pairing::{
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
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
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
    use crate::services::communication::pairing::types::PairCompletePayload;
    use crate::services::communication::pairing::{
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
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
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

/// Regression test for the P2 review finding: a probe from a peer whose
/// Bluetooth channel this device set up and then *switched off* must get no
/// reply -- replying would let that peer believe "Find via Bluetooth"
/// succeeded and record the address on its own end, only for every real
/// session attempt to then be rejected by this device's own
/// `check_channel_enabled` gate. Distinct from the never-set-up case above:
/// that one must still reply (it's the whole point of this discovery flow),
/// a switched-off channel must not. A row that exists and is off is exactly
/// what tells the two apart.
#[tokio::test(flavor = "multi_thread")]
async fn bluetooth_probe_gets_no_reply_when_the_channel_is_switched_off() {
    let (receiver, receiver_db) = server_state("channel-bluetooth-probe-switched-off");
    seed_paired_device(&receiver_db, "peer-client");
    {
        let mut conn = open_db_at_path(&receiver_db);
        channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, false, None)
            .expect("set the channel up, switched off");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gate_receiver = receiver.clone();
    let gate_db = receiver_db.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
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
        let link: Box<dyn DataLink> = Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, gate_receiver, gate_db).await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
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
/// `channel::ble` advertising flag `ble::tests` already covers
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

/// ADR-0005's opening evidence, as a test of the wiring rather than of the
/// transition table (which `link_state`'s own tests cover exhaustively).
///
/// On real hardware a Bluetooth session lapsed to amber and was still claimed
/// 1h44m later, refusing every incoming connection from that same peer while
/// its dial side wound down to exhausted. The row said connected throughout.
/// Nothing tore the dead link down, because a lapsed proof only recoloured a
/// row -- there was no transition to carry an effect.
///
/// Proves the whole chain now: a lapsed proof enters the grace window, the
/// session survives it, and once the grace expires the machine emits its
/// teardown, the session's `Close` is delivered, and the slot is genuinely
/// released so a later connection can claim it.
#[tokio::test(flavor = "multi_thread")]
async fn a_lapsed_proof_tears_the_session_down_once_grace_expires() {
    use crate::services::communication::pairing::link_state::{LinkEvent, FADE_GRACE};

    let (server, server_db) = server_state("transport-lapsed-proof-tears-down");
    seed_paired_device(&server_db, "peer-client");

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let consumer_server = server.clone();
    let consumer_db = server_db.clone();
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            if matches!(command, crate::services::communication::sync::types::SessionCommand::Close) {
                consumer_server.release_session("peer-client", ChannelKind::Network, &consumer_db);
                break;
            }
        }
    });

    assert!(server.try_claim_session("peer-client", ChannelKind::Network, tx, &server_db));
    let start = std::time::Instant::now();
    server.submit_link_event_at("peer-client", ChannelKind::Network, LinkEvent::ProofComplete, start);

    // The proof lapses. Nothing may be torn down yet: a link that goes quiet
    // for one cycle is the case the grace window exists to ride out.
    server.submit_link_event_at("peer-client", ChannelKind::Network, LinkEvent::ProofLapsed, start);
    server.submit_link_event_at(
        "peer-client",
        ChannelKind::Network,
        LinkEvent::Tick,
        start + FADE_GRACE / 2,
    );
    sleep(Duration::from_millis(50)).await;
    assert!(
        server.has_session_on("peer-client", ChannelKind::Network),
        "the session must survive the grace window, not be torn down on the first missed proof"
    );

    // Grace expires. This is the edge that did not exist.
    server.submit_link_event_at(
        "peer-client",
        ChannelKind::Network,
        LinkEvent::Tick,
        start + FADE_GRACE,
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut released = false;
    while tokio::time::Instant::now() < deadline {
        if !server.has_session_on("peer-client", ChannelKind::Network) {
            released = true;
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(
        released,
        "a session whose proof stayed lapsed past the grace window must be torn down, \
         not left holding the transport's only slot"
    );

    // The slot is genuinely free again -- the property whose absence made the
    // original failure permanent rather than merely wrong.
    let (tx2, _rx2) = tokio::sync::mpsc::channel(4);
    assert!(
        server.try_claim_session("peer-client", ChannelKind::Network, tx2, &server_db),
        "a later connection from the same peer must be able to claim the freed slot"
    );
}

/// #179. ADR-0007 promises that adding a second channel to a paired device
/// "does not interrupt the other person" — trust belongs to the pair, not
/// to a channel. Nothing carried that across the wire, so the receiving
/// side kept no row for the new channel, `run_peer_gate` answered
/// `is_enabled = false`, and every authentication on it was rejected while
/// the initiating device's dialog said the peer had nothing to do.
///
/// Here the client authenticates over Network and announces that Bluetooth
/// is now set up for this pair. The server must end up with that channel
/// configured and enabled, without anyone touching the server.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_announced_by_the_peer_is_set_up_on_the_receiving_side() {
    let (server, server_db) = server_state("transport-tcpws-channel-announce");
    seed_paired_device(&server_db, "peer-client");

    {
        let mut conn = open_db_at_path(&server_db);
        assert!(
            channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).is_none(),
            "precondition: this pair has never used Bluetooth on the receiving side"
        );
    }

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
        &PeerFrame::ChannelEnabled {
            kind: ChannelKind::Bluetooth,
        },
    )
    .await
    .expect("announce the channel");

    let mut enabled = false;
    for _ in 0..100 {
        {
            let mut conn = open_db_at_path(&server_db);
            if channels::is_enabled(&mut conn, "peer-client", ChannelKind::Bluetooth) {
                enabled = true;
                break;
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(
        enabled,
        "the peer announced Bluetooth over the channel it was already trusted on, so this \
         side must set it up itself — otherwise its own gate rejects every Bluetooth \
         authentication and the pair waits for a person to flip a second switch by hand"
    );
}
