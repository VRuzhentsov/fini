//! The Bluetooth transport: BLE GATT `Link`s over `ble-gatt`'s datagram tier
//! (github.com/VRuzhentsov/ble-gatt).
//!
//! Linux only for now (BlueZ via `ble_gatt::backend::linux`). Android needs
//! the same `tao` -> `ndk-context` bridge `tauri-plugin-ble-gatt`'s own
//! `android_lazy` module already solved for the JS-facing plugin — this
//! Rust-native path (Fini's own backend calling `ble-gatt` directly, no
//! Tauri IPC involved) hasn't been wired up for it yet. Gating the whole
//! module behind `target_os = "linux"` keeps that follow-up isolated rather
//! than half-implemented behind runtime checks.
//!
//! Plays the same "Bluetooth fallback" role `transport::sim` plays for
//! tests/E2E, but for real: network is preferred (see
//! `transport::selection`), Bluetooth engages only when a paired peer is not
//! effectively reachable over the network. Unlike `tcp_ws` (backed by the
//! mDNS/UDP presence worker) and `sim` (statically configured ports), there
//! is no discovery step here — candidates come from stored per-peer
//! Bluetooth metadata (`paired_devices.bluetooth_address`, gated on
//! `bluetooth_enabled` and a live OS-pairing check), exactly what
//! `device_connection::commands::bluetooth_address_is_os_paired` already
//! checks for the enable command.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ble_gatt::backend::linux::LinuxBackend;
use ble_gatt::datagram::{self, DatagramChannel, DatagramConfig};
use ble_gatt::{Backend, CharacteristicUuid, PeerAddress, ServiceUuid};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::services::device_connection::DeviceConnectionState;
use crate::services::space_sync::session;
use crate::services::transport::{BoxDialFuture, Link, Transport, TransportKind};

/// Fini's own GATT service/characteristic for the datagram tier. Fixed, not
/// user-configurable: both sync peers must advertise/expect the same UUIDs
/// to find each other's service. Distinct from any third-party device's own
/// UUIDs — this is Fini-to-Fini only, the app-to-app case `ble-gatt`'s
/// ADR-0003 describes.
const FINI_BLE_SERVICE_UUID: &str = "b1e6a000-f101-4000-8000-00805f9b34fb";
const FINI_BLE_CHARACTERISTIC_UUID: &str = "b1e6a001-f101-4000-8000-00805f9b34fb";

fn datagram_config() -> DatagramConfig {
    DatagramConfig::new(
        ServiceUuid(Uuid::parse_str(FINI_BLE_SERVICE_UUID).expect("valid UUID literal")),
        CharacteristicUuid(Uuid::parse_str(FINI_BLE_CHARACTERISTIC_UUID).expect("valid UUID literal")),
    )
}

/// One `LinuxBackend` for the process's lifetime. `ble_gatt::backend::linux::LinuxBackend::new()`
/// opens a BlueZ D-Bus session and requires a powered adapter; constructing
/// it lazily (on first dial/serve attempt) rather than at startup means a
/// machine with no/unpowered Bluetooth adapter never fails app startup over
/// a transport most sessions won't use.
async fn backend() -> Result<Arc<dyn Backend>, String> {
    static BACKEND: OnceCell<Result<Arc<dyn Backend>, String>> = OnceCell::const_new();
    BACKEND
        .get_or_init(|| async {
            LinuxBackend::new()
                .await
                .map(|backend| Arc::new(backend) as Arc<dyn Backend>)
                .map_err(|err| err.to_string())
        })
        .await
        .clone()
}

pub struct BleLink {
    channel: DatagramChannel,
    peer_addr: String,
}

impl BleLink {
    fn new(channel: DatagramChannel) -> Self {
        let peer_addr = channel.peer().0.clone();
        Self { channel, peer_addr }
    }
}

#[async_trait]
impl Link for BleLink {
    fn kind(&self) -> TransportKind {
        TransportKind::Bluetooth
    }

    fn peer_addr(&self) -> Option<String> {
        Some(self.peer_addr.clone())
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        self.channel.send(payload).await.map_err(|err| err.to_string())
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        match self.channel.recv().await? {
            Ok(bytes) => Some(Ok(bytes)),
            Err(err) => Some(Err(err.to_string())),
        }
    }
}

/// Central role: dial a peer's Bluetooth address.
pub async fn dial(address: &str) -> Result<Box<dyn Link>, String> {
    let backend = backend().await?;
    let peer = PeerAddress(address.to_string());
    let channel = datagram::connect(backend, &peer, &datagram_config())
        .await
        .map_err(|err| format!("ble connect to {address} failed: {err}"))?;
    Ok(Box::new(BleLink::new(channel)))
}

/// `Transport` implementation for the Bluetooth adapter — see the note on
/// `transport::tcp_ws::TcpWsTransport` for why production dial loops call
/// `dial()` directly rather than through this trait object.
#[allow(dead_code)]
pub struct BleTransport;

#[async_trait]
impl Transport for BleTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Bluetooth
    }

    fn dial(&self, _peer_device_id: &str, addr: &str, _port: u16) -> BoxDialFuture {
        let addr = addr.to_string();
        Box::pin(async move { dial(&addr).await })
    }
}

/// Peripheral role: advertise Fini's BLE service and gate every accepted
/// central through the same transport-neutral session gate every other
/// adapter uses. No-op (logs and returns) when the local adapter can't do
/// peripheral mode, or isn't available at all — Bluetooth is always a
/// fallback, never a hard requirement to start the app. `ui-plane`/`test`
/// only, matching `tcp_ws::run_server`/`sim::run_server` — `cli-plane` dials
/// out but does not run an inbound acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_server(state: DeviceConnectionState, db_path: PathBuf) {
    use futures_util::StreamExt;

    let backend = match backend().await {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("[transport][ble] adapter unavailable, Bluetooth transport disabled: {err}");
            return;
        }
    };
    if !backend.capabilities().await.peripheral {
        eprintln!("[transport][ble] adapter has no peripheral support; central-only for this device");
        return;
    }
    let mut incoming = match datagram::serve(backend, &datagram_config()).await {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("[transport][ble] advertise failed: {err}");
            return;
        }
    };
    eprintln!("[transport][ble] advertising, awaiting centrals");

    while let Some(channel) = incoming.next().await {
        let link: Box<dyn Link> = Box::new(BleLink::new(channel));
        let state = state.clone();
        let db_path = db_path.clone();
        tokio::spawn(session::run_peer_gate(link, state, db_path));
    }
}

/// Fallback dial loop: for every paired, Bluetooth-enabled, OS-paired peer
/// with no active session and no effectively-available network, attempt a
/// central-role connect. `candidates` is (peer_device_id, bluetooth_address)
/// for peers meeting those stored-metadata conditions — gathered by the
/// caller from `paired_devices` (see
/// `device_connection::commands::bluetooth_dial_candidates`), since unlike
/// `tcp_ws`/`sim` there is no presence worker or static port list to draw
/// from here.
pub fn spawn_dial_loop(
    state: &DeviceConnectionState,
    db_path: PathBuf,
    candidates: &[(String, String)],
) {
    let my_id = state.identity.device_id.clone();

    for (peer_id, address) in candidates {
        if !should_dial_peer(&my_id, peer_id, state.has_session(peer_id)) {
            continue;
        }
        // Network-first: only engage when the peer isn't effectively
        // reachable over the network — mirrors `sim::spawn_fallback_dial_loop`'s
        // `sim_is_preferred` check, but for the real fallback role instead
        // of the test stand-in.
        if state.network_effectively_available(peer_id) {
            continue;
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id = peer_id.clone();
        let address = address.clone();
        tauri::async_runtime::spawn(async move {
            dial_with_backoff(state, db_path, peer_id, address).await;
        });
    }
}

/// Deterministic dialer rule, mirroring `tcp_ws::should_dial_peer`/
/// `sim::should_dial_fallback_peer`: exactly one side of a pair ever
/// attempts to dial, so both peers dialling each other in the same tick
/// can't race to claim the same session on both ends.
fn should_dial_peer(my_id: &str, peer_id: &str, has_session: bool) -> bool {
    my_id < peer_id && !has_session
}

async fn dial_with_backoff(state: DeviceConnectionState, db_path: PathBuf, peer_id: String, address: String) {
    let mut delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(30);

    loop {
        if state.has_session(&peer_id) || state.network_effectively_available(&peer_id) {
            return;
        }

        match dial(&address).await {
            Ok(mut link) => {
                match session::perform_client_auth(link.as_mut(), &state.identity.device_id, &peer_id)
                    .await
                {
                    Ok(()) => {
                        eprintln!("[transport][ble] auth OK with {peer_id} via {address}");
                        let (tx, rx) = tokio::sync::mpsc::channel(64);
                        if state.try_claim_session(&peer_id, TransportKind::Bluetooth, tx) {
                            session::run_session(link, rx, state.clone(), db_path.clone(), peer_id.clone())
                                .await;
                            eprintln!("[transport][ble] session with {peer_id} ended");
                        }
                        delay = Duration::from_secs(2);
                    }
                    Err(err) => {
                        eprintln!("[transport][ble] auth with {peer_id} failed: {err}");
                        if err.starts_with("auth rejected") {
                            return; // not paired; don't retry
                        }
                    }
                }
            }
            Err(err) => eprintln!("[transport][ble] connect to {peer_id} ({address}) failed: {err}"),
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_dial_only_from_the_lower_device_id_and_only_without_a_session() {
        assert!(should_dial_peer("local-a", "peer-b", false));
        assert!(!should_dial_peer("peer-b", "local-a", false));
        assert!(!should_dial_peer("local-a", "peer-b", true));
        assert!(!should_dial_peer("same-id", "same-id", false));
    }
}
