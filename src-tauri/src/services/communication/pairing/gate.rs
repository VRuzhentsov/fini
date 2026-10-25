//! The inbound gate: who is allowed to open a session, and on which channel.
//!
//! This is the first thing an accepted `DataLink` meets. `run_peer_gate`
//! answers the frames before a session (pairing, adding a channel,
//! identifying a device). `run_session_gate` finds the paired device by the
//! key the TLS handshake proved, checks the pair's channel switch, claims the
//! per-(peer, channel) session slot, and only then hands the link to
//! `sync::session::run_session`.
//!
//! It lives in `pairing` because that is what it decides. It used to sit in
//! `sync::session`, where roughly a hundred and ninety lines of
//! "is this device paired, is this channel switched on" ran inside the sync
//! module -- a second place where the rules about who may connect could be
//! answered, and therefore a second place they could drift from `pairing`'s
//! answer. A channel service initiates the handshake, because it is holding
//! the link, but it does not get to decide the rules; those live here, once.
//!
//! The session loop it hands off to stays in `sync`, which is what that
//! module is for.

use std::path::PathBuf;
use std::time::Duration;

use diesel::prelude::*;

use crate::schema::paired_devices;
use crate::services::db::open_db_at_path;
use crate::services::communication::channel::{recv_frame, send_frame, DataLink};
use crate::services::communication::pairing::{ChannelKind, DeviceConnectionState};
use crate::services::communication::sync::session::{
    CLOSE_CHANNEL_OFF, CLOSE_CROSSED, CLOSE_NOT_PAIRED, CLOSE_NO_CHANNEL, CLOSE_SESSION_ACTIVE,
};
use crate::services::communication::sync::types::PeerFrame;

/// How long a new connection waits for the held session it replaces to
/// release its slot.
#[cfg(any(feature = "ui-plane", test))]
const HELD_SESSION_RELEASE_WAIT: Duration = Duration::from_secs(3);

/// Whether this device refuses a peer's inbound exchange because its own
/// dial to that peer is under way and takes precedence: the dial from the
/// smaller device id wins a crossing.
fn yields_to_own_connection(own_device_id: &str, peer_device_id: &str, connecting_to_peer: bool) -> bool {
    connecting_to_peer && own_device_id < peer_device_id
}

/// ADR-0009 D8: a session or a channel hello comes only over a link whose
/// handshake proved a key, and that key must be the one pinned for the
/// pair. A link that proves none (a plain Bluetooth link, which carries only
/// pre-pairing frames) and a pair with no key pinned (made before keys
/// existed) are both refused.
fn key_matches(db_path: &PathBuf, device_id: &str, link_key: Option<&str>) -> bool {
    let Some(link_key) = link_key else {
        return false;
    };
    let pinned = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        super::pinned_key(&mut conn, device_id)
    });
    pinned.as_deref() == Some(link_key)
}

fn check_paired(db_path: &PathBuf, device_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        paired_devices::table
            .find(device_id)
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap_or(0)
            > 0
    })
}

/// Whether this pair's channel of `kind` exists at all (`Off` or `On`):
/// its init completed here, and it has not been unlinked since.
#[cfg(any(feature = "ui-plane", test))]
fn check_channel_initialized(db_path: &PathBuf, device_id: &str, kind: ChannelKind) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        super::channels::find(&mut conn, device_id, kind).is_some()
    })
}

/// Whether this pair's channel of `kind` is set up and switched on.
///
/// Checked on every accept, in addition to `check_paired`. The dial loops
/// already enforce it on the *dialing* side, but that says nothing about the
/// *accepting* side: a peer whose own copy of this channel is still on will
/// keep dialing us, and `check_paired` alone would let it straight back in --
/// so the switch would stop our outgoing traffic and silently permit the
/// session anyway.
///
/// Observed on hardware before the Network half of this existed: turning
/// Network off left `device_connection_session_channel` reporting the network
/// seconds later, with the row showing "Off" over a live session -- precisely
/// the lie the redesign exists to remove.
///
/// `Some(true)`: the channel is on here. `Some(false)`: it is off. `None`: this
/// pair has no such channel here, so the peer removes its own (`CLOSE_NO_CHANNEL`).
///
/// Fails closed: an unreadable row reads as off, never as missing, so a read
/// error can not make the peer remove its channel.
fn channel_on_here(db_path: &PathBuf, device_id: &str, kind: ChannelKind) -> Option<bool> {
    use crate::schema::channels::dsl;
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        dsl::channels
            .find((device_id, kind.code()))
            .select(dsl::enabled)
            .first::<bool>(&mut conn)
            .optional()
            .unwrap_or_else(|err| {
                log::warn!("[space_sync][gate] reading the {kind:?} channel of {device_id} failed: {err}");
                Some(false)
            })
    })
}

/// Server-side gate for the frames before a session (`PEER_ALPN`, and every
/// plain Bluetooth link): read the first frame off a freshly accepted
/// `DataLink` and dispatch it. Pairing messages (`PairStart`/`PairAccept`/
/// `PairComplete`) are handled and the link is then closed — discovery and
/// pairing metadata are untrusted regardless of which channel carried
/// them. Sessions come through `run_session_gate`. Transport-neutral: call
/// this from every adapter's accept loop.
///
/// `ui-plane`/`test` only: gated the same way the original `ws_server`
/// listener was — `cli-plane` dials out for sync (see the adapters'
/// `spawn_dial_loop`/`spawn_fallback_dial_loop`, ungated) but does not run
/// an inbound listener/pairing acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_peer_gate(mut link: Box<dyn DataLink>, state: DeviceConnectionState, db_path: PathBuf) {
    let kind = link.kind();
    let from_addr = link.peer_addr().unwrap_or_default();
    let link_key = link.peer_key();
    let Some(Ok(frame)) = recv_frame(link.as_mut()).await else {
        return;
    };

    match frame {
        PeerFrame::PairStart(payload) => {
            let key = link_key.or_else(|| payload.from_endpoint_id.clone());
            let request_id = payload.request_id.clone();
            let via_bluetooth = kind == ChannelKind::Bluetooth;
            let _ = state.receive_ws_pair_request(payload, from_addr, key, via_bluetooth);
            if via_bluetooth {
                log::info!("[pairing][ble] parked the link request {request_id} arrived on");
                state.bluetooth_carrier.park_link(request_id.clone(), link);
                // Hold here until the request is answered or has expired.
                // The channel service counts this central as live until the
                // gate returns, and on Linux re-registers the advertisement
                // -- tearing the parked link down -- once none is. Keep the
                // add-mode scan off the adapter meanwhile too: its probes
                // dial the very device whose link is parked here.
                #[cfg(any(target_os = "linux", target_os = "android"))]
                let _leg = state.bluetooth.scanner.begin_pairing_step();
                let hold_until = tokio::time::Instant::now()
                    + Duration::from_secs(super::PAIR_REQUEST_TTL_SECS as u64 + 5);
                while tokio::time::Instant::now() < hold_until {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let pending = state.runtime.lock().map_or(false, |mut runtime| {
                        runtime.prune_expired_incoming_requests();
                        runtime.incoming_requests.contains_key(&request_id)
                    });
                    if !pending {
                        break;
                    }
                }
                log::info!("[pairing][ble] request {request_id} settled; releasing its link");
                drop(state.bluetooth_carrier.take_link(&request_id));
            }
            return;
        }
        PeerFrame::PairAccept(payload) => {
            let _ = state.receive_ws_pair_accept(payload);
            return;
        }
        PeerFrame::PairComplete(payload) => {
            let key = link_key.or_else(|| payload.from_endpoint_id.clone());
            let _ = state.receive_ws_pair_complete(
                payload,
                from_addr,
                key,
                kind == ChannelKind::Bluetooth,
            );
            return;
        }
        // ADR-0008 D1/D2: half of a channel's init. Answered while this
        // device is itself running a setup search for that peer on this
        // channel -- an init needs both people at it -- and afterwards while
        // the channel it made still exists: an ack lost on the way means the
        // peer is still asking after this side has finished. Anything else
        // (not paired, not searching and never set up, unlinked) gets silence.
        PeerFrame::AddChannel { device_id } => {
            let searching = state.channel_setup(&device_id, kind).is_some();
            if check_paired(&db_path, &device_id)
                && key_matches(&db_path, &device_id, link_key.as_deref())
                && (searching || check_channel_initialized(&db_path, &device_id, kind))
            {
                state.accept_channel_request(link.as_mut(), &device_id, kind).await;
            }
            return;
        }
        PeerFrame::IdentifyDevice { device_id, hostname, endpoint_id } => {
            // No reply at all when not in add-mode -- matching the
            // network-discovery equivalent (a mDNS beacon simply isn't
            // broadcast outside add-mode), rather than an explicit
            // rejection frame that would let a scanner distinguish "not in
            // add-mode" from "connection failed."
            if state.is_add_mode_enabled() {
                // Both devices list each other from advertisements; the
                // device that identifies us needs only our answer.
                let _ = (device_id, hostname, endpoint_id);
                let _ = send_frame(
                    link.as_mut(),
                    &PeerFrame::DeviceIdentified {
                        device_id: state.identity.device_id.clone(),
                        hostname: state.identity.hostname.clone(),
                        endpoint_id: state.identity.endpoint_id.clone(),
                    },
                )
                .await;
                // Do not drop a TCP/WebSocket connection immediately after
                // writing the reply: that can reset the stream before the
                // peer reads its reply. Wait briefly for the scanner to close.
                let _ = tokio::time::timeout(Duration::from_secs(1), link.recv()).await;
            }
            return;
        }
        other => {
            log::warn!(
                "[pairing][gate] {kind:?} link from {from_addr}: unexpected first frame `{}`",
                other.wire_type()
            );
        }
    }
}

/// Server-side gate for a sync session (`SYNC_ALPN`). There is no auth
/// frame: the TLS handshake proved the peer's key, and the paired device is
/// the one that key is pinned for (ADR-0009 D8). A refusal closes the
/// connection with a code (`CLOSE_*`). Accepted, the per-(peer, channel)
/// session slot is claimed and the session runs.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_session_gate(mut link: Box<dyn DataLink>, state: DeviceConnectionState, db_path: PathBuf) {
    let kind = link.kind();
    let from_addr = link.peer_addr().unwrap_or_default();
    let device_id = link.peer_key().and_then(|key| {
        tokio::task::block_in_place(|| super::paired_device_for_key(&mut open_db_at_path(&db_path), &key))
    });
    let Some(device_id) = device_id else {
        log::warn!("[space_sync][gate] {kind:?} session from {from_addr} refused: its key is pinned for no pair");
        link.close(CLOSE_NOT_PAIRED, "not paired; pair the devices again");
        if kind == ChannelKind::Bluetooth {
            state.bluetooth.close_rejected_link(&state, from_addr, None);
        }
        return;
    };
    log::info!("[space_sync][gate] {kind:?} session from {device_id} via {from_addr}");

    // Checked here too, not only on the connecting side: a peer whose own
    // copy of this channel is still on keeps connecting, and the switch here
    // must hold. Fails closed: no such channel, or an unreadable row, is off.
    match channel_on_here(&db_path, &device_id, kind) {
        Some(true) => {}
        Some(false) => {
            log::warn!("[space_sync][gate] {kind:?} session from {device_id} refused: the channel is off for this pair");
            link.close(CLOSE_CHANNEL_OFF, "the channel is off for this pair");
            if kind == ChannelKind::Bluetooth {
                state.bluetooth.close_rejected_link(&state, from_addr, Some(&device_id));
            }
            return;
        }
        // Removed here, or not set up yet: the peer removes its row, unless
        // it has a setup with this device open (`hold_session`).
        None => {
            log::warn!("[space_sync][gate] {kind:?} session from {device_id} refused: no such channel for this pair");
            link.close(CLOSE_NO_CHANNEL, "no such channel for this pair");
            if kind == ChannelKind::Bluetooth {
                state.bluetooth.close_rejected_link(&state, from_addr, Some(&device_id));
            }
            return;
        }
    }

    // Both devices connected to each other at once. Were both connections
    // kept and then each dropped for the other, neither session would
    // survive; the connection from the smaller device id is kept on both
    // sides.
    if yields_to_own_connection(&state.identity.device_id, &device_id, state.channel(kind).connecting(&device_id)) {
        log::info!("[space_sync][gate] {kind:?} session from {device_id} refused: this device's connection is kept");
        link.close(CLOSE_CROSSED, "crossed connections: the other device's connection is kept");
        return;
    }

    // The old session goes, and this connection replaces it, when:
    // - this device's own connection to the peer is the other half of a
    //   crossing and lost it above (the peer has the smaller id); or
    // - the session is held: it ends only when its connection drops, so a
    //   peer that connects again has restarted, and the old connection is
    //   dead.
    // Wait until the old session has released its slot, so that its release
    // cannot free the new claim.
    let held = crate::services::communication::sync::session::idle_after(kind).is_none();
    if (held || state.channel(kind).connecting(&device_id)) && state.close_session_on(&device_id, kind) {
        log::info!("[space_sync][gate] {kind:?} {device_id} connected again; closing its held session");
        let until = tokio::time::Instant::now() + HELD_SESSION_RELEASE_WAIT;
        while state.has_session_on(&device_id, kind) && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    let accepted = state
        .space_sync
        .hold_session(&state, link, &device_id, kind, || {
            log::info!("[space_sync][gate] {kind:?} session with {device_id} accepted");
        })
        .await;
    if accepted.is_ok() && kind == ChannelKind::Bluetooth {
        state.bluetooth.close_links_after_session(&state, &device_id);
    }
    if let Err(mut link) = accepted {
        // If this fires repeatedly for a peer with no other live session on
        // this channel, the claim is stale: a slot never released by a
        // session that ended without `release_session`.
        log::warn!("[space_sync][gate] {kind:?} session from {device_id} refused: a session is already active");
        link.close(CLOSE_SESSION_ACTIVE, "a session is already active on this channel");
    }
}

/// The gate as iroh's `ProtocolHandler` for one channel's endpoint: iroh's
/// `Router` accepts every connection and hands it here once the TLS
/// handshake proved the peer's key. Registered for both of Fini's ALPNs: a
/// session (`SYNC_ALPN`) goes to `run_session_gate`, the frames before one
/// (`PEER_ALPN`) to `run_peer_gate`.
#[cfg(any(feature = "ui-plane", test))]
#[derive(Clone)]
pub struct Gate {
    state: DeviceConnectionState,
    kind: ChannelKind,
}

#[cfg(any(feature = "ui-plane", test))]
impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate").field("kind", &self.kind).finish()
    }
}

#[cfg(any(feature = "ui-plane", test))]
impl Gate {
    /// Serves `endpoint` for channel `kind` until the endpoint closes: an
    /// iroh `Router` with this gate for both ALPNs. The `Router` stops when
    /// dropped, so this holds it.
    pub async fn serve(state: DeviceConnectionState, kind: ChannelKind, endpoint: iroh::Endpoint) {
        use crate::services::communication::channel::iroh_link::{PEER_ALPN, SYNC_ALPN};
        let gate = Gate { state, kind };
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(SYNC_ALPN, gate.clone())
            .accept(PEER_ALPN, gate)
            .spawn();
        endpoint.closed().await;
        let _ = router.shutdown().await;
    }

    /// Where the connection's selected path reaches the peer: an IP address
    /// for Network, a Bluetooth address for Bluetooth.
    fn peer_addr(&self, connection: &iroh::endpoint::Connection) -> Option<String> {
        let paths = connection.paths();
        let path = paths.iter().find(|path| path.is_selected())?;
        match path.remote_addr() {
            iroh::TransportAddr::Ip(addr) => Some(addr.ip().to_string()),
            #[cfg(any(target_os = "linux", target_os = "android"))]
            iroh::TransportAddr::Custom(addr) => ble_gatt_iroh::peer_address(addr).map(|peer| peer.0),
            _ => None,
        }
    }
}

#[cfg(any(feature = "ui-plane", test))]
impl iroh::protocol::ProtocolHandler for Gate {
    async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), iroh::protocol::AcceptError> {
        use crate::services::communication::channel::iroh_link::{IrohDataLink, SYNC_ALPN};
        // The test switch: a channel switched off answers nobody.
        if self.kind == ChannelKind::Network && self.state.network.switched_off() {
            connection.close(0u32.into(), b"the network is switched off");
            return Ok(());
        }
        let is_session = connection.alpn() == SYNC_ALPN;
        let peer_addr = self.peer_addr(&connection);
        let link = match IrohDataLink::accept(self.kind, connection, peer_addr).await {
            Ok(link) => Box::new(link),
            Err(err) => {
                log::warn!("[pairing][gate] {:?}: {err}", self.kind);
                return Ok(());
            }
        };
        let state = self.state.clone();
        let db_path = state.db_path.clone();
        if is_session {
            run_session_gate(link, state, db_path).await;
        } else {
            run_peer_gate(link, state, db_path).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod crossed_connection_tests {
    use super::*;

    /// Both sides apply the same rule, so exactly one of two crossed dials
    /// survives: the one from the smaller device id.
    #[test]
    fn exactly_one_of_two_crossed_connections_survives() {
        // "a" dials "b" and "b" dials "a", both at once.
        let a_refuses_b = yields_to_own_connection("a", "b", true);
        let b_refuses_a = yields_to_own_connection("b", "a", true);
        assert!(a_refuses_b, "a's own connection is kept, so it refuses b's");
        assert!(!b_refuses_a, "b accepts a's connection");
        assert!(!yields_to_own_connection("a", "b", false), "no crossing, nothing refused");
    }
}
