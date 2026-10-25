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
#[cfg(any(feature = "ui-plane", test, target_os = "linux", target_os = "android"))]
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
    /// One-shot pre-auth pairing sender (`PairStart`/`PairAccept`/
    /// `PairComplete`): one iroh connection to the device holding
    /// `peer_key`, one frame, closed once the peer has it.
    pub(crate) fn send(
        &self, state: &DeviceConnectionState, peer_key: &str, addr: IpAddr, port: u16, msg: PeerFrame,
    ) -> Result<(), String> {
        tauri::async_runtime::block_on(state.network.send_one_frame(peer_key, addr, port, &msg))
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

/// How long writing one pairing frame on an open Bluetooth link may take.
/// `send_frame` can hang on a stalled write, and all three BLE pairing legs
/// (request/accept/complete) block on it via `block_on`, so an unbounded
/// hang freezes the whole command and leaves the pairing controls stuck
/// disabled.
///
/// The dial before it bounds itself (`connect_for_pairing`): first the wait for
/// a candidate probe already in flight, then its own budget. One shared
/// deadline over both once let the wait eat the dial's time, so a dial that
/// would have connected was abandoned mid-connect.
#[cfg(any(target_os = "linux", target_os = "android"))]
const PAIR_FRAME_SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// The link a Bluetooth `PairStart` arrived on, kept open by request id so
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
            // Keeps the add-mode candidate scan off the adapter and out of
            // this peer's dial until the frame is sent.
            let (mut link, _clear_of_scan) = state.bluetooth.connect_for_pairing(state, peer_key, address).await?;
            send_pair_frame(link.as_mut(), &msg).await
        })
    }

    /// Sends a Bluetooth `PairStart` over iroh, to the key the peer's
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
        let peer_key = state
            .bluetooth.scanner.available
            .key(to_device_id)
            .ok_or_else(|| "that device has not been found over Bluetooth yet -- try again".to_string())?;
        // The picker's address can be stale: Android re-advertises from a
        // new private address on every add-mode change.
        let latest = state.bluetooth.scanner.available.latest_address(to_device_id);
        let address = latest.as_deref().unwrap_or(address);
        let link = tauri::async_runtime::block_on(async {
            let (mut link, _clear_of_scan) = state.bluetooth.connect_for_pairing(state, &peer_key, address).await?;
            send_pair_frame(link.as_mut(), &msg).await?;
            Ok::<_, String>(link)
        })?;
        listen_for_answers(state.clone(), link, to_device_id.to_string(), address.to_string());
        Ok(())
    }
}

/// Writes one pairing frame within `PAIR_FRAME_SEND_TIMEOUT`.
#[cfg(any(target_os = "linux", target_os = "android"))]
async fn send_pair_frame(link: &mut dyn DataLink, msg: &PeerFrame) -> Result<(), String> {
    tokio::time::timeout(PAIR_FRAME_SEND_TIMEOUT, crate::services::communication::channel::send_frame(link, msg))
        .await
        .map_err(|_| "bluetooth pairing send timed out".to_string())?
}

/// Reads the accept and the complete that come back on a request's link.
#[cfg(all(any(target_os = "linux", target_os = "android"), any(feature = "ui-plane", test)))]
fn listen_for_answers(state: DeviceConnectionState, mut link: Box<dyn DataLink>, to_device_id: String, address: String) {
    use crate::services::communication::channel::recv_frame;
    tauri::async_runtime::spawn(async move {
        // The whole exchange runs on this one link, so keep the add-mode
        // scan from probing the peer -- a second connection to the same
        // device -- until it is over.
        let _leg = state.bluetooth.scanner.begin_pairing_step();
        let peer_key = link.peer_key().and_then(|key| key.parse::<iroh::EndpointId>().ok());
        let listen_for = Duration::from_secs(PAIR_REQUEST_TTL_SECS as u64 + 30);
        let mut completed = false;
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
                        completed = true;
                        break;
                    }
                    _ => log::warn!("[pairing][ble] unexpected frame on the request's link"),
                }
            }
        })
        .await;
        drop(link);
        log::info!("[pairing][ble] request link closed");
        let Some(peer_key) = peer_key else {
            return;
        };
        if completed {
            // The first session goes over this pairing's BLE link, which is
            // still up: no search and no new connect (`exchange_with`).
            state.bluetooth.keep_pairing_link(&state, &to_device_id, &address, peer_key);
        } else {
            // The phone leaves Add Device with a new address; the BLE link to
            // its old one must not outlive the pairing.
            state
                .bluetooth
                .close_links_to(peer_key, crate::services::communication::channel::bluetooth::LINK_CLOSE_DELAY)
                .await;
        }
    });
}

impl PairingCarrier for BluetoothCarrier {
    /// Answers a Bluetooth `PairStart` on the iroh connection it arrived
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
                    Ok(Err(err)) => log::warn!("[pairing][ble] request link failed: {err}; connecting instead"),
                    Err(_) => log::warn!("[pairing][ble] request link timed out; connecting instead"),
                }
            }
            None => log::warn!("[pairing][ble] no parked link for {request_id}; connecting instead"),
        }
        let peer_key = to.key.ok_or_else(|| "the requester's key is unknown".to_string())?;
        self.send(state, peer_key, to.addr, msg)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn reply(&self, _state: &DeviceConnectionState, _to: &ReplyTo<'_>, _msg: PeerFrame, _more: bool) -> Result<(), String> {
        Err("Bluetooth is not available on this platform".to_string())
    }
}
