//! How a pairing frame reaches the other device: over the channel its
//! request came on.
//!
//! | | request | answers (accept, complete) |
//! |---|---|---|
//! | `NetworkCarrier` | one iroh connection per frame, to the key the peer's beacon announced | the same, back to the requester |
//! | `BluetoothCarrier` | one iroh connection over Bluetooth, held open for the whole exchange | on the link the request arrived on, parked by the gate |

use std::net::IpAddr;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::time::Duration;

use super::DeviceConnectionState;
#[cfg(all(any(target_os = "linux", target_os = "android"), any(feature = "ui-plane", test)))]
use super::PAIR_REQUEST_TTL_SECS;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::communication::channel::DataLink;
use crate::services::communication::sync::types::PeerFrame;

/// Where the answers to an incoming request go: back to its requester.
pub(crate) struct ReplyTo<'a> {
    pub request_id: &'a str,
    /// Where the request came from.
    pub addr: &'a str,
    /// The requester's Network port.
    pub ws_port: u16,
    /// The requester's iroh key, if it sent one.
    pub key: Option<&'a str>,
}

pub(crate) trait PairingCarrier: Send + Sync {
    /// Sends `msg`, an answer to an incoming request. `more` says another
    /// answer to the same request follows (the accept, before the
    /// complete).
    fn reply(&self, state: &DeviceConnectionState, to: &ReplyTo<'_>, msg: PeerFrame, more: bool) -> Result<(), String>;
}

/// Pairing over the Network channel.
pub(crate) struct NetworkCarrier;

impl NetworkCarrier {
    /// One-shot pre-auth pairing sender (`PairRequest`/`PairAccept`/
    /// `PairComplete`): one iroh connection to the device holding
    /// `peer_key`, one frame, closed once the peer has it.
    pub(crate) fn send(
        &self, state: &DeviceConnectionState, peer_key: &str, addr: IpAddr, port: u16, msg: PeerFrame,
    ) -> Result<(), String> {
        tauri::async_runtime::block_on(crate::services::communication::channel::network::send_one_frame(
            state, peer_key, addr, port, &msg,
        ))
    }
}

impl PairingCarrier for NetworkCarrier {
    fn reply(&self, state: &DeviceConnectionState, to: &ReplyTo<'_>, msg: PeerFrame, _more: bool) -> Result<(), String> {
        let target_ip: IpAddr = to
            .addr
            .parse()
            .map_err(|err| format!("invalid sender addr '{}': {err}", to.addr))?;
        let peer_key = to.key.ok_or_else(|| "the requester's key is unknown".to_string())?;
        self.send(state, peer_key, target_ip, to.ws_port, msg)
    }
}

/// A stale/unresponsive BLE candidate has no bound of its own here: `dial`
/// can hang trying to connect to a device that's since gone out of range,
/// and `send_frame` can hang on a stalled write. All three BLE pairing legs
/// (request/accept/complete) block on this via `block_on`, so an unbounded
/// hang here freezes the whole command -- leaving pairing controls stuck
/// disabled, and letting a retry after the request TTL expires collide with
/// the still-open earlier attempt.
///
/// Sized to include the wait `dial_for_pairing` may spend letting a
/// candidate probe already in flight finish before its own dial: at 10s the
/// wait alone consumed half the budget and the dial was abandoned
/// mid-connect.
///
/// The sum it has to hold, worst case, when Pair is pressed just as a probe
/// starts dialling a slow advertiser:
///
///   `CANDIDATE_PROBE_TIMEOUT`   25s   waiting for that probe's lock
///   `PAIRING_DIAL_RETRY_WINDOW`  5s   then retrying its own dial, the last
///   ble-gatt `CONNECT_TIMEOUT`  20s   attempt of which may run to the bound
///
/// so ~50s before a frame is even written. Capped under
/// `PAIR_REQUEST_TTL_SECS` (60s) deliberately: past the TTL the request is
/// dead anyway, and a retry would collide with this still-open attempt --
/// the collision the paragraph above exists to prevent. Any of those three
/// growing has to come with a look at this one.
#[cfg(any(target_os = "linux", target_os = "android"))]
const SEND_PAIR_BLE_TIMEOUT: Duration = Duration::from_secs(55);

/// The link a Bluetooth `PairRequest` arrived on, kept open by request id so
/// the accept and the complete answer on it rather than dialling the
/// requester back -- see `BluetoothCarrier::send_request` for why.
#[cfg(any(feature = "ui-plane", test))]
type ParkedLinks = std::sync::Mutex<std::collections::HashMap<String, Box<dyn DataLink>>>;

/// Pairing over Bluetooth (ADR 0002 Phase 3, ADR-0009 D7). One per
/// `DeviceConnectionState`: it holds the links requests arrived on.
#[derive(Default)]
pub(crate) struct BluetoothCarrier {
    #[cfg(any(feature = "ui-plane", test))]
    parked: ParkedLinks,
}

impl BluetoothCarrier {
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn park_link(&self, request_id: String, link: Box<dyn DataLink>) {
        if let Ok(mut links) = self.parked.lock() {
            links.insert(request_id, link);
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn take_link(&self, request_id: &str) -> Option<Box<dyn DataLink>> {
        self.parked.lock().ok()?.remove(request_id)
    }

    /// One-shot pre-auth pairing sender over Bluetooth: dials `peer_key`
    /// over iroh, starting from `address`, and sends one frame.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn send(&self, state: &DeviceConnectionState, peer_key: &str, address: &str, msg: PeerFrame) -> Result<(), String> {
        tauri::async_runtime::block_on(async move {
            tokio::time::timeout(SEND_PAIR_BLE_TIMEOUT, async {
                // Keeps the add-mode candidate scan off the adapter and out
                // of this peer's dial until the frame is sent.
                let (mut link, _clear_of_scan) =
                    crate::services::communication::channel::bluetooth::dial_for_pairing(state, peer_key, address)
                        .await?;
                crate::services::communication::channel::send_frame(link.as_mut(), &msg).await
            })
            .await
            .map_err(|_| "bluetooth pairing send timed out".to_string())?
        })
    }

    /// Sends a Bluetooth `PairRequest` over iroh, to the key the peer's
    /// add-mode hello reported, and keeps that one connection open, reading
    /// the accept and the complete that come back on it until the request
    /// has expired. Nothing is dialled back to this device: Android connects
    /// out from a private address it never advertises on. The initiator
    /// holds the connection, iroh carries the whole exchange on it --
    /// retransmitting and re-dialling the link underneath -- as sessions
    /// already do (ADR-0009).
    #[cfg(all(any(target_os = "linux", target_os = "android"), any(feature = "ui-plane", test)))]
    pub(crate) fn send_request(
        &self, state: &DeviceConnectionState, to_device_id: &str, address: &str, msg: PeerFrame,
    ) -> Result<(), String> {
        use crate::services::communication::channel::send_frame;
        let peer_key = state
            .bluetooth_peers
            .key(to_device_id)
            .ok_or_else(|| "that device has not been found over Bluetooth yet -- try again".to_string())?;
        // The picker's address can be stale: Android re-advertises from a
        // new private address on every add-mode change.
        let latest = state.bluetooth_peers.latest_address(to_device_id);
        let address = latest.as_deref().unwrap_or(address);
        let link = tauri::async_runtime::block_on(async {
            tokio::time::timeout(SEND_PAIR_BLE_TIMEOUT, async {
                let (mut link, _clear_of_scan) =
                    crate::services::communication::channel::bluetooth::dial_for_pairing(state, &peer_key, address)
                        .await?;
                send_frame(link.as_mut(), &msg).await?;
                Ok::<_, String>(link)
            })
            .await
            .map_err(|_| "bluetooth pairing send timed out".to_string())?
        })?;
        listen_for_answers(state.clone(), link);
        Ok(())
    }
}

/// Reads the accept and the complete that come back on a request's link.
#[cfg(all(any(target_os = "linux", target_os = "android"), any(feature = "ui-plane", test)))]
fn listen_for_answers(state: DeviceConnectionState, mut link: Box<dyn DataLink>) {
    use crate::services::communication::channel::recv_frame;
    tauri::async_runtime::spawn(async move {
        // The whole exchange runs on this one link, so keep the add-mode
        // scan from probing the peer -- a second connection to the same
        // device -- until it is over.
        let _leg = state.bluetooth_radio.begin_pairing_leg();
        let listen_for = Duration::from_secs(PAIR_REQUEST_TTL_SECS as u64 + 30);
        let _ = tokio::time::timeout(listen_for, async {
            while let Some(Ok(frame)) = recv_frame(link.as_mut()).await {
                match frame {
                    PeerFrame::PairAccept(payload) => {
                        log::info!("[pairing][ble] accept arrived on the request's link");
                        let _ = state.receive_ws_pair_accept(payload);
                    }
                    PeerFrame::PairComplete(payload) => {
                        log::info!("[pairing][ble] complete arrived on the request's link");
                        let from_addr = link.peer_addr().unwrap_or_default();
                        let key = link.peer_key().or_else(|| payload.from_endpoint_id.clone());
                        let _ = state.receive_ws_pair_complete(payload, from_addr, key, true);
                        break;
                    }
                    _ => log::warn!("[pairing][ble] unexpected frame on the request's link"),
                }
            }
        })
        .await;
        log::info!("[pairing][ble] request link closed");
    });
}

impl PairingCarrier for BluetoothCarrier {
    /// Answers a Bluetooth `PairRequest` on the iroh connection it arrived
    /// on, which the gate parked (see `send_request`). `more` parks it again
    /// for the next answer. With no parked connection, or a dead one, it
    /// dials the requester's key over iroh, starting from where the request
    /// came from.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn reply(&self, state: &DeviceConnectionState, to: &ReplyTo<'_>, msg: PeerFrame, more: bool) -> Result<(), String> {
        let request_id = to.request_id;
        // The CLI has no gate, so nothing is ever parked there.
        #[cfg(not(any(feature = "ui-plane", test)))]
        let _ = (request_id, more);
        #[cfg(any(feature = "ui-plane", test))]
        match self.take_link(request_id) {
            Some(mut link) => {
                let sent = tauri::async_runtime::block_on(tokio::time::timeout(
                    Duration::from_secs(10),
                    crate::services::communication::channel::send_frame(link.as_mut(), &msg),
                ));
                match sent {
                    Ok(Ok(())) => {
                        log::info!("[pairing][ble] answered {request_id} on the request's link");
                        if more {
                            self.park_link(request_id.to_string(), link);
                        } else {
                            // `send` returning only means queued: hold the
                            // link until the requester closes it after
                            // reading.
                            tauri::async_runtime::block_on(async {
                                let _ = tokio::time::timeout(Duration::from_secs(15), link.recv()).await;
                            });
                        }
                        return Ok(());
                    }
                    Ok(Err(err)) => log::warn!("[pairing][ble] request link failed: {err}; dialling instead"),
                    Err(_) => log::warn!("[pairing][ble] request link timed out; dialling instead"),
                }
            }
            None => log::warn!("[pairing][ble] no parked link for {request_id}; dialling instead"),
        }
        let peer_key = to.key.ok_or_else(|| "the requester's key is unknown".to_string())?;
        self.send(state, peer_key, to.addr, msg)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn reply(&self, _state: &DeviceConnectionState, _to: &ReplyTo<'_>, _msg: PeerFrame, _more: bool) -> Result<(), String> {
        Err("Bluetooth is not available on this platform".to_string())
    }
}
