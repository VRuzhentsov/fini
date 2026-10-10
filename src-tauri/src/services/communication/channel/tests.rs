//! End-to-end proof that the transport abstraction works: two independent
//! adapters (`network`, the Bluetooth stub) carry the exact same `crate::services::communication::pairing::run_peer_gate`/
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
use crate::services::communication::channel::{network, recv_frame, send_frame, DataLink, Transport, ChannelKind};

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

        /// Stands in for the key the real Bluetooth session's TLS proves:
        /// the test client's (ADR-0009 D8).
        fn peer_key(&self) -> Option<String> {
            Some(super::test_client_key().public().to_string())
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

/// The key every test client dials with. Fixed, so a seeded pair can pin it
/// before the client exists (ADR-0009 D8).
fn test_client_key() -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[7; 32])
}

/// Dials `server`'s Network endpoint on `port` as the test client. The
/// client's state is leaked so its endpoint outlives the call, as a real
/// device's does.
async fn dial_to(server: &DeviceConnectionState, port: u16) -> Result<Box<dyn DataLink>, String> {
    let (mut client, _) = server_state_on_port("test-client", 0);
    client.secret_key = test_client_key();
    let link = network::dial(&client, &server.identity.endpoint_id, "127.0.0.1".parse().unwrap(), port).await;
    std::mem::forget(client);
    link
}

fn seed_paired_device(db_path: &PathBuf, peer_device_id: &str) {
    seed_paired_device_with_key(db_path, peer_device_id, &test_client_key().public().to_string());
}

/// Seeds a pair whose peer holds `endpoint_id` (ADR-0009 D8).
fn seed_paired_device_with_key(db_path: &PathBuf, peer_device_id: &str, endpoint_id: &str) {
    let mut conn = open_db_at_path(db_path);
    diesel::insert_into(paired_devices::table)
        .values(&CreatePairedDeviceInput {
            peer_device_id: peer_device_id.to_string(),
            display_name: "Test Peer".to_string(),
            paired_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .execute(&mut conn)
        .expect("seed paired device");
    diesel::update(paired_devices::table.find(peer_device_id))
        .set(paired_devices::endpoint_id.eq(endpoint_id))
        .execute(&mut conn)
        .expect("pin the peer's key");
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
    // The same database the test seeds, as in the app: code that opens the
    // state's own `db_path` (exchanges, setup) must see those rows.
    let state = DeviceConnectionState::from_db_path(&data_dir, db_path.clone());
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

/// A state that only dials falls back to any free port when its own is
/// taken (the CLI next to a running app). A serving state does not: peers
/// dial the port presence announces, so it must sit there or nowhere.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_dialing_state_falls_back_to_a_free_port() {
    let taken = std::net::UdpSocket::bind(("0.0.0.0", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();

    let (dialer, _) = server_state_on_port("network-port-dialer", port);
    let endpoint = network::endpoint(&dialer).await.expect("a dialer binds some port");
    assert_ne!(network::bound_port(&endpoint), port);

    let (server, _) = server_state_on_port("network-port-server", port);
    server.serves_network.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(network::endpoint(&server).await.is_err(), "a server must not move off its port");

    drop(taken);
    let endpoint = network::endpoint(&server).await.expect("the port is free again");
    assert_eq!(network::bound_port(&endpoint), port);
}

#[tokio::test(flavor = "multi_thread")]
async fn network_gate_accepts_paired_device_and_claims_session_as_network() {
    let (server, server_db) = server_state("transport-tcpws-accept");
    seed_paired_device(&server_db, "peer-client");
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
        .await
        .expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth should succeed for paired device");

    sleep(Duration::from_millis(50)).await;
    assert!(server.has_session_on("peer-client", ChannelKind::Network));
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
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
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
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
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
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
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

    // Bounded wait for the server to handle the frame, not a guess.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let row = loop {
        let row = channels::find(&mut open_db_at_path(&server_db), "peer-client", ChannelKind::Bluetooth)
            .expect("the Bluetooth channel row");
        if row.address.is_some() || tokio::time::Instant::now() >= deadline {
            break row;
        }
        sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(row.address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));

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
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
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
async fn network_gate_rejects_unpaired_device() {
    let (server, _server_db) = server_state("transport-tcpws-reject");
    let port = network::spawn_server_on_free_port(server.clone(), server.db_path.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
        .await
        .expect("dial");
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("unpaired device should be rejected");
    assert!(err.contains("auth rejected"), "unexpected error: {err}");
}

/// ADR-0009 D8: the key a connection proves must be the one pinned for the
/// pair. A device that knows a paired device's id but holds another key is
/// refused, which the UUID alone never did.
#[tokio::test(flavor = "multi_thread")]
async fn network_gate_rejects_a_paired_device_id_presented_with_another_key() {
    let (server, server_db) = server_state("network-key-mismatch");
    seed_paired_device(&server_db, "peer-client");
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;

    let (impostor, _) = server_state_on_port("network-key-mismatch-impostor", 0);
    let mut link = network::dial(&impostor, &server.identity.endpoint_id, "127.0.0.1".parse().unwrap(), port)
        .await
        .expect("dial");
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("another key must be refused");
    assert!(err.contains("key does not match"), "unexpected error: {err}");
    assert!(!server.has_session_on("peer-client", ChannelKind::Network));
}

/// ADR-0009 D8: a pair made before keys existed has none pinned, and is
/// refused until it is paired again.
#[tokio::test(flavor = "multi_thread")]
async fn network_gate_rejects_a_pair_with_no_key_pinned() {
    let (server, server_db) = server_state("network-no-key");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        diesel::update(paired_devices::table.find("peer-client"))
            .set(paired_devices::endpoint_id.eq(None::<String>))
            .execute(&mut conn)
            .expect("unpin");
    }
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;

    let mut link = dial_to(&server, port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("a pair with no key pinned must be refused");
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
    assert!(server.has_session_on("peer-client", ChannelKind::Bluetooth));
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
    let loopback_port = free_port().await;
    let tcp_port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), loopback_port));
    sleep(Duration::from_millis(100)).await;

    let mut first_link = dial_to(&server, tcp_port)
        .await
        .expect("dial network");
    session::perform_client_auth(first_link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("first session should authenticate");
    sleep(Duration::from_millis(50)).await;
    assert!(server.has_session_on("peer-client", ChannelKind::Network));

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
        server.active_exchange_channel("peer-client"),
        Some(ChannelKind::Network),
        "traffic goes over Network while both are connected"
    );

    drop(first_link);
    drop(second_link);
}

/// The Bluetooth adapter implements the `Transport` port polymorphically.
/// The Network channel no longer does: its dial needs this device's iroh
/// endpoint and the peer's key, which a `Transport` cannot carry.
#[tokio::test(flavor = "multi_thread")]
async fn both_adapters_satisfy_the_transport_port() {
    let (server, server_db) = server_state("transport-polymorphic");
    seed_paired_device(&server_db, "peer-client");
    let loopback_port = free_port().await;
    tokio::spawn(bluetooth_stub::run_server(server.clone(), server_db.clone(), loopback_port));
    sleep(Duration::from_millis(100)).await;

    let adapters: Vec<(Box<dyn Transport>, u16, ChannelKind)> = vec![
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

/// What a one-shot pairing frame produced on the receiving side, once it has
/// been handled: waits (bounded) instead of guessing how long that takes.
async fn arrived<T>(read: impl Fn() -> Vec<T>) -> Vec<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let items = read();
        if !items.is_empty() || tokio::time::Instant::now() >= deadline {
            return items;
        }
        sleep(Duration::from_millis(20)).await;
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

    let _add_mode_guard = super::bluetooth::ADD_MODE_TEST_LOCK.lock().unwrap();
    let (receiver, receiver_db) = server_state("transport-send-pair-request-receiver");
    device_connection_enter_add_mode_impl(&receiver, true).expect("enter add mode");
    let port = network::spawn_server_on_free_port(receiver.clone(), receiver_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let (sender, _sender_db) = server_state_on_port("transport-send-pair-request-sender", 0);
    let sender_device_id = sender.identity.device_id.clone();
    let receiver_device_id = receiver.identity.device_id.clone();
    // No presence heard, as for a one-shot CLI command: the key is given.
    let receiver_key = receiver.identity.endpoint_id.clone();
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
                to_endpoint_id: Some(receiver_key),
            },
        )
        .expect("send pair request");
    })
    .await
    .expect("join send-pair-request task");

    let incoming = arrived(|| device_connection_pair_incoming_requests_impl(&receiver).expect("list incoming requests")).await;
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

    let _add_mode_guard = super::bluetooth::ADD_MODE_TEST_LOCK.lock().unwrap();
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
    device_connection_enter_add_mode_impl(&requester, true).expect("enter add mode (requester)");
    device_connection_enter_add_mode_impl(&accepter, true).expect("enter add mode (accepter)");

    tokio::spawn(network::run_server_on_port(
        requester.clone(),
        requester_db.clone(),
        requester_port,
    ));
    tokio::spawn(network::run_server_on_port(
        accepter.clone(),
        accepter_db.clone(),
        accepter_port,
    ));
    sleep(Duration::from_millis(100)).await;

    let accepter_device_id = accepter.identity.device_id.clone();
    requester.note_presence_for_test(
        &accepter_device_id,
        "127.0.0.1",
        accepter_port,
        Some(accepter.identity.endpoint_id.clone()),
    );
    let requester_for_send = requester.clone();
    tokio::task::spawn_blocking(move || {
        device_connection_send_pair_request_impl(
            &requester_for_send,
            DevicePairRequestInput {
                request_id: "req-round-trip".to_string(),
                to_device_id: accepter_device_id,
                to_addr: "127.0.0.1".to_string(),
                to_ws_port: Some(accepter_port),
                to_endpoint_id: None,
            },
        )
        .expect("send pair request");
    })
    .await
    .expect("join send-pair-request task");

    let incoming = arrived(|| device_connection_pair_incoming_requests_impl(&accepter).expect("list incoming requests")).await;
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

    let outgoing = arrived(|| device_connection_pair_outgoing_updates_impl(&requester).expect("list outgoing updates")).await;
    assert_eq!(
        outgoing.len(),
        1,
        "requester should receive the pair code back from the accepter"
    );
    assert_eq!(outgoing[0].request_id, "req-round-trip");
    assert!(outgoing[0].code.chars().all(|ch| ch.is_ascii_digit()));
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

    fn peer_key(&self) -> Option<String> {
        self.0.peer_key()
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
    assert!(server.has_session_on("peer-client", ChannelKind::Bluetooth));

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

    let _add_mode_guard = super::bluetooth::ADD_MODE_TEST_LOCK.lock().unwrap();
    let (receiver, receiver_db) = server_state("transport-pair-request-bluetooth");
    device_connection_enter_add_mode_impl(&receiver, true).expect("enter add mode");

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
            from_endpoint_id: None,
            to_device_id: receiver.identity.device_id.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            expires_at: "2099-01-01T00:00:00Z".to_string(),
        }),
    )
    .await
    .expect("send pair request over bluetooth-kind link");

    let incoming = arrived(|| device_connection_pair_incoming_requests_impl(&receiver).expect("list incoming requests")).await;
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
            from_endpoint_id: None,
        }),
    )
    .await
    .expect("send pair complete over bluetooth-kind link");

    let completions = arrived(|| device_connection_pair_outgoing_completions_impl(&receiver).expect("list outgoing completions")).await;
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
    let port = network::spawn_server_on_free_port(receiver.clone(), receiver_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&receiver, port)
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
            from_endpoint_id: None,
        }),
    )
    .await
    .expect("send pair complete over network");

    let completions = arrived(|| device_connection_pair_outgoing_completions_impl(&receiver).expect("list outgoing completions")).await;
    assert_eq!(completions.len(), 1);
    assert!(!completions[0].via_bluetooth);
    assert_eq!(
        completions[0].bluetooth_address.as_deref(),
        Some("11:22:33:44:55:66")
    );
}

/// Sends a hello from `from_device_id` to this device's gate and returns
/// whatever came back within a short window (`None` for silence).
async fn say_hello(
    state: DeviceConnectionState,
    db_path: PathBuf,
    from_device_id: &str,
) -> Option<PeerFrame> {
    let mut link = dial_bluetooth_gate(state, db_path).await;
    send_frame(
        link.as_mut(),
        &PeerFrame::Hello {
            device_id: from_device_id.to_string(),
        },
    )
    .await
    .expect("the hello itself sends fine -- the peer is reachable");
    match tokio::time::timeout(Duration::from_millis(800), recv_frame(link.as_mut())).await {
        Ok(Some(Ok(frame))) => Some(frame),
        _ => None,
    }
}

/// ADR-0008 D1/D2: while this device runs a setup search for a paired peer,
/// it acknowledges that peer's hello -- and records that half of the init.
#[tokio::test(flavor = "multi_thread")]
async fn a_hello_is_acknowledged_while_this_device_searches_for_the_peer() {
    let (receiver, receiver_db) = server_state("adr-0008-hello-while-searching");
    seed_paired_device(&receiver_db, "peer-client");
    receiver.begin_channel_setup("peer-client", ChannelKind::Bluetooth);

    match say_hello(receiver.clone(), receiver_db, "peer-client").await {
        Some(PeerFrame::HelloAck { device_id }) => {
            assert_eq!(device_id, receiver.identity.device_id);
        }
        other => panic!("expected a HelloAck, got {other:?}"),
    }
    // The gate records its half only once the ack is sent, so the test can
    // read the ack a moment before the record lands.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let setup = loop {
        let setup = receiver
            .channel_setup("peer-client", ChannelKind::Bluetooth)
            .expect("the search is still running");
        if setup.acked_peer_hello || tokio::time::Instant::now() >= deadline {
            break setup;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(setup.acked_peer_hello, "this device's half of the init is recorded");
    assert!(!setup.initialized(), "the other half is the peer acknowledging our hello");
}

/// ADR-0008 D2: a device that is not searching and never set this channel
/// up says nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_hello_goes_unanswered_when_this_device_is_not_searching() {
    let (receiver, receiver_db) = server_state("adr-0008-hello-not-searching");
    seed_paired_device(&receiver_db, "peer-client");
    assert!(
        say_hello(receiver, receiver_db, "peer-client").await.is_none(),
        "a device that is not searching must stay silent"
    );
}

/// ADR-0008 D2: once init has made the channel, its hellos are still acked,
/// `Off` or `On`. This side's ack can be lost when the link drops, so the
/// peer is still searching after this side has finished; without an answer
/// it could never complete the same setup.
#[tokio::test(flavor = "multi_thread")]
async fn a_hello_is_acknowledged_for_a_channel_that_already_exists() {
    for enabled in [false, true] {
        let (receiver, receiver_db) = server_state("adr-0008-hello-existing-channel");
        seed_paired_device(&receiver_db, "peer-client");
        {
            let mut conn = open_db_at_path(&receiver_db);
            channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, enabled, None)
                .expect("set the channel up");
        }
        match say_hello(receiver.clone(), receiver_db, "peer-client").await {
            Some(PeerFrame::HelloAck { device_id }) => {
                assert_eq!(device_id, receiver.identity.device_id, "channel on={enabled}")
            }
            other => panic!("channel on={enabled}: expected a HelloAck, got {other:?}"),
        }
    }
}

/// A hello from a device this one never paired gets no reply, even while it
/// is searching for someone else.
#[tokio::test(flavor = "multi_thread")]
async fn a_hello_from_an_unpaired_device_goes_unanswered() {
    let (receiver, receiver_db) = server_state("adr-0008-hello-unpaired");
    seed_paired_device(&receiver_db, "peer-client");
    receiver.begin_channel_setup("peer-client", ChannelKind::Bluetooth);
    receiver.begin_channel_setup("a-stranger", ChannelKind::Bluetooth);

    assert!(say_hello(receiver, receiver_db, "a-stranger").await.is_none());
}

/// ADR-0008 D15: finishing a setup writes the channel only when its init
/// completed -- `On` for OK, `Off` for closing the dialog -- and nothing
/// when it did not.
#[test]
fn finishing_a_setup_writes_the_channel_only_after_a_complete_init() {
    let (state, db_path) = server_state("adr-0008-finish-setup");
    seed_paired_device(&db_path, "peer-client");
    let mut conn = open_db_at_path(&db_path);

    state.begin_channel_setup("peer-client", ChannelKind::Bluetooth);
    state.note_channel_setup("peer-client", ChannelKind::Bluetooth, |s| s.acked_peer_hello = true);
    crate::services::communication::pairing::setup::finish(
        &mut conn, &state, "peer-client", ChannelKind::Bluetooth, true,
    )
    .expect("finish a half-done init");
    assert!(
        channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).is_none(),
        "half an init is no init: nothing is written"
    );

    // Closing creates the channel `Off`; OK switches it on; closing a later
    // setup (the code fallback may have set it up meanwhile) leaves it `On`.
    for (switch_on, expected) in [(false, false), (true, true), (false, true)] {
        state.begin_channel_setup("peer-client", ChannelKind::Bluetooth);
        state.note_channel_setup("peer-client", ChannelKind::Bluetooth, |s| {
            s.acked_peer_hello = true;
            s.hello_acked_by_peer = true;
        });
        crate::services::communication::pairing::setup::finish(
            &mut conn, &state, "peer-client", ChannelKind::Bluetooth, switch_on,
        )
        .expect("finish a complete init");
        assert_eq!(
            channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).map(|c| c.enabled),
            Some(expected),
        );
        assert!(state.channel_setup("peer-client", ChannelKind::Bluetooth).is_none());
    }
}

/// A setup closed and reopened for the same channel is a new attempt: what
/// the closed one's search brings back late does not count for it.
#[test]
fn a_late_result_from_a_closed_setup_does_not_count_for_the_next() {
    let (state, _db) = server_state("adr-0008-setup-attempts");
    let closed = state.begin_channel_setup("peer-client", ChannelKind::Bluetooth);
    assert_eq!(state.begin_channel_setup("peer-client", ChannelKind::Bluetooth), closed, "joins the running one");
    state.end_channel_setup("peer-client", ChannelKind::Bluetooth);

    let reopened = state.begin_channel_setup("peer-client", ChannelKind::Bluetooth);
    assert_ne!(reopened, closed);
    state.note_channel_setup_attempt("peer-client", ChannelKind::Bluetooth, closed, |s| {
        s.hello_acked_by_peer = true
    });
    assert!(!state.channel_setup("peer-client", ChannelKind::Bluetooth).unwrap().hello_acked_by_peer);

    state.note_channel_setup_attempt("peer-client", ChannelKind::Bluetooth, reopened, |s| {
        s.hello_acked_by_peer = true
    });
    assert!(state.channel_setup("peer-client", ChannelKind::Bluetooth).unwrap().hello_acked_by_peer);
}

/// ADR-0008 D9: a peer whose Network beacons stopped stops being present
/// once the channel timeout passes.
#[test]
fn network_presence_expires_after_the_channel_timeout() {
    let (state, _db) = server_state("adr-0008-network-presence-expiry");
    state.note_presence_for_test("peer-client", "127.0.0.1", 1, None);
    assert!(state.network_peer_available("peer-client"));
    assert!(state.network_presence_address("peer-client").is_some());

    state.age_presence_for_test(
        "peer-client",
        crate::services::communication::pairing::NETWORK_CHANNEL_TIMEOUT,
    );
    assert!(!state.network_peer_available("peer-client"));
    assert!(state.network_presence_address("peer-client").is_none());
}

/// A completed init survives a failed write of its channel, so pressing OK
/// again works without both people repeating the setup.
#[test]
fn a_completed_setup_survives_a_failed_channel_write() {
    let (state, db_path) = server_state("adr-0008-finish-write-fails");
    // No paired device: the channel row cannot be written.
    let mut conn = open_db_at_path(&db_path);
    state.begin_channel_setup("peer-client", ChannelKind::Bluetooth);
    state.note_channel_setup("peer-client", ChannelKind::Bluetooth, |s| {
        s.acked_peer_hello = true;
        s.hello_acked_by_peer = true;
    });

    crate::services::communication::pairing::setup::finish(
        &mut conn, &state, "peer-client", ChannelKind::Bluetooth, true,
    )
    .expect_err("the write fails");
    assert!(state
        .channel_setup("peer-client", ChannelKind::Bluetooth)
        .is_some_and(|s| s.initialized()));

    seed_paired_device(&db_path, "peer-client");
    crate::services::communication::pairing::setup::finish(
        &mut conn, &state, "peer-client", ChannelKind::Bluetooth, true,
    )
    .expect("the retry writes it");
    assert_eq!(
        channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).map(|c| c.enabled),
        Some(true)
    );
    assert!(state.channel_setup("peer-client", ChannelKind::Bluetooth).is_none());
}

/// ADR-0007: the primary is the person's choice of channel, shown on its
/// row. Only a channel that is on can be chosen, and switching the chosen
/// one off releases the choice.
#[test]
fn the_primary_channel_is_chosen_shown_and_released_when_switched_off() {
    use crate::services::communication::pairing::{
        device_connection_set_channel_enabled_impl, device_connection_set_primary_channel_impl,
    };
    let (state, db_path) = server_state("adr-0007-primary-channel");
    seed_paired_device(&db_path, "peer-client");
    let mut conn = open_db_at_path(&db_path);
    channels::configure(&mut conn, "peer-client", ChannelKind::Network, true, None).unwrap();
    channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, false, None).unwrap();

    let err = device_connection_set_primary_channel_impl(
        &mut conn, &state, "peer-client".to_string(), Some(ChannelKind::Bluetooth),
    )
    .expect_err("an Off channel cannot be the primary");
    assert!(err.contains("Switch the channel on first"), "unexpected error: {err}");

    let rows = device_connection_set_primary_channel_impl(
        &mut conn, &state, "peer-client".to_string(), Some(ChannelKind::Network),
    )
    .expect("choose Network");
    let starred: Vec<_> = rows.iter().filter(|row| row.primary).map(|row| row.kind).collect();
    assert_eq!(starred, vec![ChannelKind::Network]);
    assert_eq!(channels::primary_kind(&mut conn, "peer-client"), Some(ChannelKind::Network));

    device_connection_set_channel_enabled_impl(
        &mut conn, &state, "peer-client".to_string(), ChannelKind::Network, false,
    )
    .expect("switch Network off");
    assert_eq!(channels::primary_kind(&mut conn, "peer-client"), None, "off releases the primary");

    device_connection_set_channel_enabled_impl(
        &mut conn, &state, "peer-client".to_string(), ChannelKind::Network, true,
    )
    .expect("switch Network on");
    device_connection_set_primary_channel_impl(
        &mut conn, &state, "peer-client".to_string(), Some(ChannelKind::Network),
    )
    .expect("choose Network again");
    let rows = device_connection_set_primary_channel_impl(&mut conn, &state, "peer-client".to_string(), None)
        .expect("clear the choice");
    assert!(rows.iter().all(|row| !row.primary), "no primary: automatic selection");
}

/// ADR-0008 D15: a channel that does not exist is set up, not switched on.
#[test]
fn switching_on_a_channel_that_was_never_set_up_is_refused() {
    let (state, db_path) = server_state("adr-0008-switch-on-none");
    seed_paired_device(&db_path, "peer-client");
    let mut conn = open_db_at_path(&db_path);

    let err = crate::services::communication::pairing::device_connection_set_channel_enabled_impl(
        &mut conn,
        &state,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
        true,
    )
    .expect_err("None cannot be switched on");
    assert!(err.contains("Set the channel up first"), "unexpected error: {err}");
    assert!(channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).is_none());
}

/// A `DiscoveryHello` from a prober that is not the server under test.
fn test_hello() -> PeerFrame {
    PeerFrame::DiscoveryHello {
        device_id: "prober-device".to_string(),
        hostname: "prober".to_string(),
        endpoint_id: "prober-key".to_string(),
    }
}

/// Regression test for Phase 3 of ADR 0002: `DiscoveryHello` only gets a
/// reply when the receiver is actually in add-mode -- the BLE-scan
/// equivalent of network discovery simply not broadcasting outside
/// add-mode. Uses `set_add_mode_for_test` (instance-scoped) rather than the
/// real `enter_add_mode_impl`, which would also flip the process-global
/// `channel::bluetooth` advertising flag `ble::tests` already covers
/// separately.
#[tokio::test(flavor = "multi_thread")]
async fn discovery_hello_gets_a_reply_only_when_the_receiver_is_in_add_mode() {
    let (server, server_db) = server_state("transport-discovery-hello-on");
    server.set_add_mode_for_test(true);
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
        .await
        .expect("dial");
    send_frame(link.as_mut(), &test_hello())
        .await
        .expect("send discovery hello");
    match recv_frame(link.as_mut()).await {
        Some(Ok(PeerFrame::DiscoveryHelloReply { device_id, hostname, endpoint_id })) => {
            assert_eq!(device_id, server.identity.device_id);
            assert_eq!(hostname, server.identity.hostname);
            assert_eq!(endpoint_id, server.identity.endpoint_id);
        }
        other => panic!("expected a DiscoveryHelloReply while in add-mode, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_hello_gets_no_reply_when_the_receiver_is_not_in_add_mode() {
    let (server, server_db) = server_state("transport-discovery-hello-off");
    // Add-mode is off by default -- no set_add_mode_for_test call.
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port)
        .await
        .expect("dial");
    send_frame(link.as_mut(), &test_hello())
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

/// ADR-0008 D14: a peer that unlinked a channel says so, and this side
/// removes its own row for it -- whatever its switch said.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_the_peer_unlinked_is_removed_here() {
    let (server, server_db) = server_state("adr-0008-peer-unlinked");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, true, None)
            .expect("Bluetooth set up and on here");
    }

    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth over the network channel");
    send_frame(link.as_mut(), &PeerFrame::ChannelUnlinked { kind: ChannelKind::Bluetooth })
        .await
        .expect("tell the server Bluetooth was unlinked");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let gone = {
            let mut conn = open_db_at_path(&server_db);
            channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).is_none()
        };
        if gone {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the pair is broken for this channel, so this side's row goes too"
        );
        sleep(Duration::from_millis(20)).await;
    }
}

/// ADR-0008 D14: unlinking tells the peer. The notice is kept in the
/// database, not in memory, so a restart before the next exchange does not
/// lose it; that exchange delivers it and it is gone.
#[tokio::test(flavor = "multi_thread")]
async fn unlinking_keeps_the_notice_until_an_exchange_delivers_it() {
    let (server, server_db) = server_state("adr-0008-unlink-push");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        channels::configure(&mut conn, "peer-client", ChannelKind::Network, true, None)
            .expect("Network set up and on");
        channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, false, None)
            .expect("Bluetooth set up, off");
        crate::services::communication::pairing::device_connection_unlink_channel_impl(
            &mut conn,
            &server,
            "peer-client".to_string(),
            ChannelKind::Bluetooth,
        )
        .expect("unlink Bluetooth");
    }
    assert!(!server.has_queued_frames("peer-client"), "nothing is held only in memory");
    assert_eq!(kept_frames(&server_db, "peer-client").len(), 1, "the notice waits in the database");

    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;

    let mut link = dial_to(&server, port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth over the network channel");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, recv_frame(link.as_mut())).await {
            Ok(Some(Ok(PeerFrame::ChannelUnlinked { kind }))) => {
                assert_eq!(kind, ChannelKind::Bluetooth);
                break;
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("expected the kept ChannelUnlinked, got {other:?}"),
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !kept_frames(&server_db, "peer-client").is_empty() && tokio::time::Instant::now() < deadline {
        sleep(Duration::from_millis(20)).await;
    }
    assert!(kept_frames(&server_db, "peer-client").is_empty(), "delivered, so no longer kept");
}

fn kept_frames(db_path: &PathBuf, peer: &str) -> Vec<PeerFrame> {
    let mut conn = open_db_at_path(db_path);
    crate::services::communication::sync::control_outbox::waiting(&mut conn, peer)
        .into_iter()
        .map(|(_, frame)| frame)
        .collect()
}

/// An unlink that is refused leaves nothing for the peer either: the row and
/// its notice are written together or not at all.
#[test]
fn a_refused_unlink_tells_the_peer_nothing() {
    let (server, server_db) = server_state("adr-0008-unlink-refused");
    seed_paired_device(&server_db, "peer-client");
    let mut conn = open_db_at_path(&server_db);
    channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, true, None).expect("set up, on");
    let err = crate::services::communication::pairing::device_connection_unlink_channel_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
    )
    .expect_err("an On channel cannot be unlinked");
    assert_eq!(err, "Turn the channel off first");
    assert!(channels::find(&mut conn, "peer-client", ChannelKind::Bluetooth).is_some());
    assert!(kept_frames(&server_db, "peer-client").is_empty());
}

/// A channel set up again before the peer heard of its unlink: the old
/// notice no longer holds, and delivered late it would delete the new one.
#[test]
fn setting_a_channel_up_again_drops_its_undelivered_unlink() {
    let (server, server_db) = server_state("adr-0008-unlink-superseded");
    seed_paired_device(&server_db, "peer-client");
    let mut conn = open_db_at_path(&server_db);
    channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, false, None).expect("set up");
    crate::services::communication::pairing::device_connection_unlink_channel_impl(
        &mut conn,
        &server,
        "peer-client".to_string(),
        ChannelKind::Bluetooth,
    )
    .expect("unlink");
    assert_eq!(kept_frames(&server_db, "peer-client").len(), 1);

    channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, false, None).expect("set up again");
    assert!(kept_frames(&server_db, "peer-client").is_empty());
}

/// A frame kept while an exchange is already running goes over that
/// exchange, not the next one.
#[tokio::test(flavor = "multi_thread")]
async fn a_frame_kept_during_an_exchange_goes_over_it() {
    let (server, server_db) = server_state("adr-0008-kept-live");
    seed_paired_device(&server_db, "peer-client");
    {
        let mut conn = open_db_at_path(&server_db);
        channels::configure(&mut conn, "peer-client", ChannelKind::Network, true, None).expect("Network on");
    }
    let port = network::spawn_server_on_free_port(server.clone(), server_db.clone()).await;
    sleep(Duration::from_millis(100)).await;
    let mut link = dial_to(&server, port).await.expect("dial");
    session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect("auth");

    {
        let mut conn = open_db_at_path(&server_db);
        crate::services::communication::sync::control_outbox::keep(
            &mut conn,
            "peer-client",
            &PeerFrame::SpaceSyncEnd { space_id: "space-1".into(), ended_at: "2026-01-01T00:00:00Z".into() },
        )
        .expect("keep");
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, recv_frame(link.as_mut())).await {
            Ok(Some(Ok(PeerFrame::SpaceSyncEnd { space_id, .. }))) => {
                assert_eq!(space_id, "space-1");
                break;
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("expected the kept SpaceSyncEnd over the running exchange, got {other:?}"),
        }
    }
}

// ADR-0008 reproductions. Adapted from the handoff's hardware-driven repros
// to the accepted model: a channel is `None`, `Off` or `On` (D15), and a
// peer whose channel is not `On` refuses the exchange. Unlinking is still a
// tombstone in the schema until D14 lands; these assert the behaviour the
// person sees, so they hold across that change.

/// Runs this device's gate for one inbound Bluetooth-kind link and returns
/// the dialling side of it.
async fn dial_bluetooth_gate(state: DeviceConnectionState, db_path: PathBuf) -> Box<dyn DataLink> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn DataLink> =
            Box::new(AsBluetooth(Box::new(bluetooth_stub::StubDataLink::new(stream))));
        crate::services::communication::pairing::run_peer_gate(link, state, db_path).await;
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    Box::new(bluetooth_stub::StubDataLink::new(stream))
}

/// Sets up this pair's Bluetooth channel switched off, and unlinks it when
/// asked -- the `Off` and removed states of ADR-0008 D15.
fn seed_bluetooth_channel_off(db_path: &PathBuf, unlink: bool) {
    let mut conn = open_db_at_path(db_path);
    channels::configure(&mut conn, "peer-client", ChannelKind::Bluetooth, false, None)
        .expect("set Bluetooth up, switched off");
    if unlink {
        channels::unlink(&mut conn, "peer-client", ChannelKind::Bluetooth).expect("unlink it");
    }
}

/// D15: an `Off` channel refuses the exchange, and says why on the wire.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0008_an_off_channel_refuses_the_exchange() {
    let (server, server_db) = server_state("adr-0008-off-refuses");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_channel_off(&server_db, false);

    let mut link = dial_bluetooth_gate(server.clone(), server_db).await;
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("an Off channel must refuse");
    assert!(err.contains("bluetooth disabled for this pair"), "unexpected refusal: {err}");
}

/// D14/D15: a channel the person unlinked refuses the exchange, exactly as
/// one that never existed.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0008_an_unlinked_channel_refuses_the_exchange() {
    let (server, server_db) = server_state("adr-0008-unlinked-refuses");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_channel_off(&server_db, true);

    let mut link = dial_bluetooth_gate(server.clone(), server_db).await;
    let err = session::perform_client_auth(link.as_mut(), "peer-client", &server.identity.device_id)
        .await
        .expect_err("an unlinked channel must refuse");
    assert!(err.contains("bluetooth disabled for this pair"), "unexpected refusal: {err}");
}

/// Inventory 4.3: an unlinked channel must not confirm a setup search. It
/// used to answer "Find", so the searching side was told "found" by a peer
/// that would refuse every exchange after it.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0008_an_unlinked_channel_does_not_answer_a_hello() {
    let (server, server_db) = server_state("adr-0008-unlinked-hello");
    seed_paired_device(&server_db, "peer-client");
    seed_bluetooth_channel_off(&server_db, true);

    assert!(say_hello(server, server_db, "peer-client").await.is_none());
}

// ADR-0008 D10: exchanges replace held sessions.

/// An exchange with nothing moving in either direction closes on its own,
/// and releases its slot.
#[tokio::test(flavor = "multi_thread")]
async fn an_exchange_closes_once_nothing_moves() {
    let (server, server_db) = server_state("adr-0008-exchange-idle");
    seed_paired_device(&server_db, "peer-client");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let exchange_state = server.clone();
    tokio::spawn(async move {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        let link: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        assert!(exchange_state.try_claim_session("peer-client", ChannelKind::Bluetooth, tx));
        session::run_session(
            link,
            rx,
            exchange_state.clone(),
            server_db,
            "peer-client".to_string(),
            crate::services::communication::sync::types::PROTOCOL_VERSION,
            Duration::from_millis(200),
        )
        .await;
    });

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut client: Box<dyn DataLink> = Box::new(bluetooth_stub::StubDataLink::new(stream));
    sleep(Duration::from_millis(50)).await;
    assert!(server.has_session_on("peer-client", ChannelKind::Bluetooth));

    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match client.recv().await {
                None | Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "an idle exchange must close the link");
    sleep(Duration::from_millis(50)).await;
    assert!(
        !server.has_session_on("peer-client", ChannelKind::Bluetooth),
        "a closed exchange releases its slot"
    );
}

/// Frames queued while the peer was away that a new exchange's mailbox
/// cannot all take stay queued, in order, ahead of anything queued later.
#[test]
fn frames_a_new_exchange_cannot_take_stay_queued_in_order() {
    let (state, _db) = server_state("adr-0008-claim-overflow");
    let frame = |space: &str| PeerFrame::BootstrapStart { space_id: space.to_string() };
    for space in ["first", "second", "third"] {
        state.queue_for_peer("peer-client", frame(space));
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    assert!(state.try_claim_session("peer-client", ChannelKind::Network, tx));
    state.queue_for_peer("peer-client", frame("later"));
    state.release_session("peer-client", ChannelKind::Network);

    let (next_tx, mut next_rx) = tokio::sync::mpsc::channel(8);
    assert!(state.try_claim_session("peer-client", ChannelKind::Network, next_tx));
    let spaces = |rx: &mut tokio::sync::mpsc::Receiver<crate::services::communication::sync::types::SessionCommand>| {
        let mut out = Vec::new();
        while let Ok(crate::services::communication::sync::types::SessionCommand::Forward(
            PeerFrame::BootstrapStart { space_id },
        )) = rx.try_recv()
        {
            out.push(space_id);
        }
        out
    };
    assert_eq!(spaces(&mut rx), ["first"]);
    assert_eq!(spaces(&mut next_rx), ["second", "third", "later"]);
    assert!(!state.has_queued_frames("peer-client"));
}

/// A frame that finds the running exchange's mailbox full waits for the next
/// exchange, which gets it first -- it is neither dropped nor left behind.
#[test]
fn a_frame_the_running_exchange_cannot_take_goes_to_the_next_one() {
    let (state, _db) = server_state("adr-0008-queue-full-mailbox");
    let frame = |space: &str| PeerFrame::BootstrapStart { space_id: space.to_string() };

    let (busy_tx, mut busy_rx) = tokio::sync::mpsc::channel(1);
    assert!(state.try_claim_session("peer-client", ChannelKind::Network, busy_tx));
    state.queue_for_peer("peer-client", frame("taken"));
    state.queue_for_peer("peer-client", frame("left over"));
    assert!(busy_rx.try_recv().is_ok(), "the first frame went to the running exchange");
    assert!(state.has_queued_frames("peer-client"), "the second waits");

    state.release_session("peer-client", ChannelKind::Network);
    assert!(state.has_queued_frames("peer-client"), "ending the exchange keeps it");

    let (next_tx, mut next_rx) = tokio::sync::mpsc::channel(4);
    assert!(state.try_claim_session("peer-client", ChannelKind::Network, next_tx));
    match next_rx.try_recv() {
        Ok(crate::services::communication::sync::types::SessionCommand::Forward(
            PeerFrame::BootstrapStart { space_id },
        )) => assert_eq!(space_id, "left over"),
        other => panic!("the next exchange did not get the frame first: {other:?}"),
    }
    assert!(!state.has_queued_frames("peer-client"));
}

/// Switching a channel off while its exchange's mailbox is full still ends
/// the exchange: the close waits for the next slot, ahead of anything
/// forwarded after it.
#[tokio::test(flavor = "multi_thread")]
async fn switching_off_a_busy_exchange_still_closes_it() {
    use crate::services::communication::sync::types::SessionCommand;
    let (state, _db) = server_state("adr-0008-close-busy");
    let frame = |space: &str| PeerFrame::BootstrapStart { space_id: space.to_string() };

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    assert!(state.try_claim_session("peer-client", ChannelKind::Bluetooth, tx));
    assert!(state.push_to_peer("peer-client", frame("filling")));
    assert!(state.close_session_on("peer-client", ChannelKind::Bluetooth));
    let later = state.clone();
    let late = tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        later.push_to_peer("peer-client", frame("late"))
    });

    assert!(matches!(rx.recv().await, Some(SessionCommand::Forward(_))));
    sleep(Duration::from_millis(100)).await;
    assert!(
        matches!(rx.recv().await, Some(SessionCommand::Close)),
        "the close is delivered before a frame forwarded after it"
    );
    drop(rx);
    let _ = late.await;
}

/// A frame raised while no exchange runs waits for the next one, and the
/// keeper starts that exchange over Network once the peer is present --
/// without any held session.
#[tokio::test(flavor = "multi_thread")]
async fn queued_work_opens_an_exchange_that_delivers_it() {
    let (receiver, receiver_db) = server_state("adr-0008-queued-receiver");
    let (sender, sender_db) = server_state("adr-0008-queued-sender");
    seed_paired_device_with_key(&receiver_db, &sender.identity.device_id, &sender.identity.endpoint_id);
    seed_paired_device_with_key(&sender_db, &receiver.identity.device_id, &receiver.identity.endpoint_id);

    let port = network::spawn_server_on_free_port(receiver.clone(), receiver_db.clone()).await;
    sleep(Duration::from_millis(100)).await;
    sender.note_presence_for_test(
        &receiver.identity.device_id,
        "127.0.0.1",
        port,
        Some(receiver.identity.endpoint_id.clone()),
    );

    sender.queue_for_peer(
        &receiver.identity.device_id,
        PeerFrame::SpaceMappingUpdate {
            mapped_space_ids: vec!["1".to_string()],
            custom_spaces: Vec::new(),
            sent_at: "2026-09-27T00:00:00Z".to_string(),
        },
    );
    assert!(sender.has_queued_frames(&receiver.identity.device_id));

    {
        let mut conn = open_db_at_path(&sender_db);
        crate::services::communication::sync::commands::space_sync_tick_impl(&mut conn, &sender)
            .expect("tick");
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let status = crate::services::communication::pairing::device_connection_debug_status_impl(&receiver)
            .expect("receiver status");
        if status.incoming_space_mapping_update_count == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the queued mapping update must arrive through an exchange the tick started"
        );
        sleep(Duration::from_millis(50)).await;
    }
    assert!(!sender.has_queued_frames(&receiver.identity.device_id));
}

/// ADR-0008 D19 over Network: an `On` channel is green while the peer's
/// presence beacon is current, grey without it, and orange when this device
/// cannot announce itself; `Off` and `None` are their own rows.
#[tokio::test(flavor = "multi_thread")]
async fn channel_rows_follow_state_presence_and_problem() {
    use crate::services::communication::pairing::channel_status::{ChannelColor, ChannelState};

    let (state, db_path) = server_state("adr-0008-rows");
    seed_paired_device(&db_path, "peer-client");
    let mut conn = open_db_at_path(&db_path);
    let row = |conn: &mut SqliteConnection, kind: ChannelKind| {
        crate::services::communication::pairing::device_connection_channel_statuses_impl(
            conn,
            &state,
            "peer-client".to_string(),
        )
        .expect("statuses")
        .into_iter()
        .find(|status| status.kind == kind)
        .expect("a row per kind")
    };

    let network = row(&mut conn, ChannelKind::Network);
    assert_eq!((network.state, network.color), (ChannelState::On, ChannelColor::Grey));

    state.note_presence_for_test("peer-client", "127.0.0.1", 1, None);
    assert_eq!(row(&mut conn, ChannelKind::Network).color, ChannelColor::Green);

    channels::set_enabled(&mut conn, "peer-client", ChannelKind::Network, false).expect("off");
    assert_eq!(row(&mut conn, ChannelKind::Network).color, ChannelColor::Off);

    let bluetooth = row(&mut conn, ChannelKind::Bluetooth);
    assert_eq!((bluetooth.state, bluetooth.color), (ChannelState::None, ChannelColor::None));
}

/// Synchronous Tauri commands (`space_sync_tick`, `watch_presence`) run on
/// the main thread, outside any Tokio runtime. What they start must not
/// need one: `tokio::spawn` there panics, and every later invoke hangs --
/// which is how the whole actors e2e lane timed out.
#[test]
fn exchanges_and_setups_start_from_outside_a_tokio_runtime() {
    let (state, db_path) = server_state("sync-command-spawn");
    seed_paired_device(&db_path, "peer-sync-command");

    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::services::communication::channel::bluetooth::start_exchange(&state, "peer-sync-command");
    crate::services::communication::pairing::setup::start(&state, "peer-sync-command", ChannelKind::Network);

    assert!(state.channel_setup("peer-sync-command", ChannelKind::Network).is_some());
    state.end_channel_setup("peer-sync-command", ChannelKind::Network);
}

/// Polls `done` until it holds, for up to `within`.
#[cfg(any(target_os = "linux", target_os = "android"))]
async fn holds_within(within: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while !done() {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(20)).await;
    }
    true
}

/// In a Bluetooth setup both devices hear each other and would dial at once,
/// and crossing dials fail each other. The device with the lower id dials at
/// once; the one with the higher id waits until the other's hello has
/// reached it, and dials anyway once its patience runs out.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[tokio::test(flavor = "multi_thread")]
async fn in_a_bluetooth_setup_the_higher_id_dials_after_the_peers_hello() {
    use crate::services::communication::channel::bluetooth::wait_for_turn_to_dial;

    let (state, _db) = server_state("setup-dial-order");
    // Device ids are UUIDs: "~" sorts after any of them, "!" before.
    let (above_me, below_me) = ("~", "!");

    tokio::time::timeout(Duration::from_millis(100), wait_for_turn_to_dial(&state, above_me))
        .await
        .expect("the lower id dials at once");

    state.begin_channel_setup(below_me, ChannelKind::Bluetooth);
    let turn = {
        let state = state.clone();
        tokio::spawn(async move { wait_for_turn_to_dial(&state, below_me).await })
    };
    sleep(Duration::from_millis(300)).await;
    assert!(!turn.is_finished(), "the higher id waits for the peer's hello");
    state.note_channel_setup(below_me, ChannelKind::Bluetooth, |setup| setup.acked_peer_hello = true);
    tokio::time::timeout(Duration::from_millis(500), turn)
        .await
        .expect("the higher id dials once the peer's hello reached it")
        .unwrap();

    state.end_channel_setup(below_me, ChannelKind::Bluetooth);
    state.begin_channel_setup(below_me, ChannelKind::Bluetooth);
    tokio::time::timeout(Duration::from_secs(3), wait_for_turn_to_dial(&state, below_me))
        .await
        .expect("without the peer's hello the higher id still dials, once its patience runs out");
    state.end_channel_setup(below_me, ChannelKind::Bluetooth);
}

/// The candidate scan stands aside for a Bluetooth setup from start to end,
/// not only until this device's hello is acknowledged: the peer's hello
/// still has to reach this device, and the scan dialling the peer meanwhile
/// keeps that from happening.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[tokio::test(flavor = "multi_thread")]
async fn a_bluetooth_setup_keeps_the_candidate_scan_aside_until_it_ends() {
    use crate::services::communication::channel::bluetooth::{pairing_legs_held, PAIRING_LEGS_TEST_LOCK};
    use crate::services::communication::pairing::setup;

    let _legs = PAIRING_LEGS_TEST_LOCK.lock().await;
    let (state, db_path) = server_state("setup-holds-pairing-leg");
    seed_paired_device(&db_path, "peer-leg");

    setup::start(&state, "peer-leg", ChannelKind::Bluetooth);
    assert!(holds_within(Duration::from_secs(2), || pairing_legs_held() == 1).await, "the setup holds a leg");

    // This device's hello was acknowledged: its own hello round is over.
    state.note_channel_setup("peer-leg", ChannelKind::Bluetooth, |setup| setup.hello_acked_by_peer = true);
    sleep(Duration::from_secs(1)).await;
    assert_eq!(pairing_legs_held(), 1, "the leg outlasts this device's own hello");

    state.end_channel_setup("peer-leg", ChannelKind::Bluetooth);
    assert!(
        holds_within(Duration::from_secs(2), || pairing_legs_held() == 0).await,
        "the leg is released when the setup ends"
    );
}
