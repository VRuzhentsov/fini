//! The inbound gate: who is allowed to open a session, and on which channel.
//!
//! This is the first thing an accepted `DataLink` meets. It answers the
//! pre-auth pairing frames, checks an `Auth` frame against `paired_devices`
//! and the pair's channel switches, claims the per-(peer, channel) session
//! slot, and only then hands the link to `sync::session::run_session`.
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
use tokio::sync::mpsc;

use crate::schema::paired_devices;
use crate::services::db::open_db_at_path;
use crate::services::communication::channel::{recv_frame, send_frame, DataLink};
use crate::services::communication::pairing::{channels, ChannelKind, DeviceConnectionState};
use crate::services::communication::sync::session::{
    run_session, CROSSED_DIAL_REASON, SESSION_ACTIVE_REASON,
};
use crate::services::communication::sync::types::{PeerFrame, SessionCommand, PROTOCOL_VERSION};

/// Whether this device refuses a peer's inbound exchange because its own
/// dial to that peer is under way and takes precedence: the dial from the
/// smaller device id wins a crossing.
fn yields_to_own_dial(own_device_id: &str, peer_device_id: &str, dialing_peer: bool) -> bool {
    dialing_peer && own_device_id < peer_device_id
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
/// Network off left `device_connection_session_channel` reporting `tcp_ws`
/// seconds later, with the row showing "Off" over a live session -- precisely
/// the lie the redesign exists to remove.
///
/// Fails closed: a pair with no such channel configured, or an unreadable
/// row, must not be treated as enabled.
fn check_channel_enabled(db_path: &PathBuf, device_id: &str, kind: ChannelKind) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        channels::is_enabled(&mut conn, device_id, kind)
    })
}

/// Server-side gate: read the first frame off a freshly accepted `DataLink` and
/// dispatch it. Pre-auth pairing messages (`PairRequest`/`PairAccept`/
/// `PairComplete`) are handled and the link is then closed — discovery and
/// pairing metadata are untrusted regardless of which channel carried
/// them. An `Auth` frame is checked against `paired_devices`; on success the
/// per-(peer, channel) session slot is claimed via `try_claim_session`
/// before `AuthOk` is sent and the session loop starts -- refused only if a
/// session is already claimed on this *same* channel for this peer, not
/// because a session already exists on the other one. Transport-neutral:
/// call this from every adapter's accept loop.
///
/// `ui-plane`/`test` only: gated the same way the original `ws_server`
/// listener was — `cli-plane` dials out for sync (see the adapters'
/// `spawn_dial_loop`/`spawn_fallback_dial_loop`, ungated) but does not run
/// an inbound listener/pairing acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_peer_gate(mut link: Box<dyn DataLink>, state: DeviceConnectionState, db_path: PathBuf) {
    let kind = link.kind();
    let from_addr = link.peer_addr().unwrap_or_default();
    let Some(Ok(frame)) = recv_frame(link.as_mut()).await else {
        return;
    };

    let (device_id, peer_device_id, peer_protocol_version) = match frame {
        PeerFrame::PairRequest(payload) => {
            let _ = state.receive_ws_pair_request(payload, from_addr, kind == ChannelKind::Bluetooth);
            return;
        }
        PeerFrame::PairAccept(payload) => {
            let _ = state.receive_ws_pair_accept(payload);
            return;
        }
        PeerFrame::PairComplete(payload) => {
            let _ =
                state.receive_ws_pair_complete(payload, from_addr, kind == ChannelKind::Bluetooth);
            return;
        }
        // ADR-0008 D1/D2: half of a channel's init. Answered only while
        // this device is itself running a setup search for that peer on
        // this channel -- an init needs both people at it. Anything else
        // (not paired, not searching, switched off, unlinked) gets silence.
        PeerFrame::Hello { device_id } => {
            if check_paired(&db_path, &device_id) && state.channel_setup(&device_id, kind).is_some() {
                if send_frame(
                    link.as_mut(),
                    &PeerFrame::HelloAck {
                        device_id: state.identity.device_id.clone(),
                    },
                )
                .await
                .is_ok()
                {
                    state.note_channel_setup(&device_id, kind, |setup| setup.acked_peer_hello = true);
                }
                // Keep the link open briefly so the ack is read before the
                // stream closes, as the `DiscoveryHello` arm below does.
                let _ = tokio::time::timeout(Duration::from_secs(1), link.recv()).await;
            }
            return;
        }
        PeerFrame::DiscoveryHello => {
            // No reply at all when not in add-mode -- matching the
            // network-discovery equivalent (a mDNS beacon simply isn't
            // broadcast outside add-mode), rather than an explicit
            // rejection frame that would let a scanner distinguish "not in
            // add-mode" from "connection failed."
            if state.is_add_mode_enabled() {
                let _ = send_frame(
                    link.as_mut(),
                    &PeerFrame::DiscoveryHelloReply {
                        device_id: state.identity.device_id.clone(),
                        hostname: state.identity.hostname.clone(),
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
        PeerFrame::Auth {
            device_id,
            peer_device_id,
            protocol_version,
        } => (device_id, peer_device_id, protocol_version),
        other => {
            log::warn!(
                "[space_sync][gate] {kind:?} link from {from_addr}: expected auth first, got `{}`",
                other.wire_type()
            );
            let _ = send_frame(
                link.as_mut(),
                &PeerFrame::AuthFail {
                    reason: "expected auth first".into(),
                },
            )
            .await;
            return;
        }
    };

    log::info!("[space_sync][gate] {kind:?} auth attempt from {device_id} via {from_addr}");

    if peer_device_id != state.identity.device_id {
        log::warn!(
            "[space_sync][gate] {kind:?} auth from {device_id} rejected: wrong target device \
             ({peer_device_id}, expected {})",
            state.identity.device_id
        );
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "wrong target device".into(),
            },
        )
        .await;
        return;
    }

    if !check_paired(&db_path, &device_id) {
        log::warn!("[space_sync][gate] {kind:?} auth from {device_id} rejected: unknown device (not paired)");
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "unknown device".into(),
            },
        )
        .await;
        return;
    }

    // Sim stands in for Bluetooth's role in tests, so it is deliberately not
    // gated here -- only the real network channel is.
    if kind == ChannelKind::Network && !check_channel_enabled(&db_path, &device_id, ChannelKind::Network) {
        log::warn!(
            "[space_sync][gate] network auth from {device_id} rejected: network disabled for this pair"
        );
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "network disabled for this pair".into(),
            },
        )
        .await;
        return;
    }

    if kind == ChannelKind::Bluetooth {
        if !check_channel_enabled(&db_path, &device_id, ChannelKind::Bluetooth) {
            log::warn!(
                "[space_sync][gate] bluetooth auth from {device_id} rejected: bluetooth disabled for this pair"
            );
            let _ = send_frame(
                link.as_mut(),
                &PeerFrame::AuthFail {
                    reason: "bluetooth disabled for this pair".into(),
                },
            )
            .await;
            return;
        }
        // ADR-0006 removed a second check here: the connecting link's
        // observed address had to match a stored one that was also currently
        // OS-bonded. It was never a trust check -- the `Auth` frame above is
        // and remains the trust boundary -- and it rejected exactly the
        // connections this channel now depends on, since a peer that
        // advertises under a rotating address never matches a stored one.
    }

    // Both devices dialed each other at once. Were both links kept and
    // then each dropped for the other, neither exchange would survive; the
    // dial from the smaller device id wins on both sides.
    if kind == ChannelKind::Network
        && yields_to_own_dial(
            &state.identity.device_id,
            &device_id,
            crate::services::communication::channel::tcp_ws::dialing(&device_id),
        )
    {
        log::info!("[space_sync][gate] network auth from {device_id} refused: this device's dial wins");
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: CROSSED_DIAL_REASON.into(),
            },
        )
        .await;
        return;
    }

    let (tx, rx) = mpsc::channel::<SessionCommand>(64);
    if !state.try_claim_session(&device_id, kind, tx) {
        // If this fires repeatedly for a peer that has no other live session
        // on this channel (check `device_connection_debug_status` / the
        // sibling session logs), the claim is stale -- a slot never released
        // by a prior session that ended without going through
        // `release_session` (e.g. a panic in `run_session`'s inbound-frame
        // handling). That's a "stuck connecting forever" shape from the
        // gate's own side, not just the dial loop's.
        log::warn!(
            "[space_sync][gate] {kind:?} auth from {device_id} rejected: session already active on this channel"
        );
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: SESSION_ACTIVE_REASON.into(),
            },
        )
        .await;
        return;
    }

    if send_frame(
        link.as_mut(),
        &PeerFrame::AuthOk {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await
    .is_err()
    {
        log::warn!("[space_sync][gate] {kind:?} auth OK for {device_id} but AuthOk send failed; releasing claim");
        state.release_session(&device_id, kind);
        return;
    }

    log::info!("[space_sync][gate] {kind:?} auth OK for {device_id}; starting exchange");
    run_session(
        link,
        rx,
        state,
        db_path,
        device_id,
        peer_protocol_version,
        crate::services::communication::sync::session::EXCHANGE_IDLE,
    )
    .await;
}

#[cfg(test)]
mod crossed_dial_tests {
    use super::*;
    use crate::services::communication::sync::session::refused_for_running_exchange;

    /// Both sides apply the same rule, so exactly one of two crossed dials
    /// survives: the one from the smaller device id.
    #[test]
    fn exactly_one_of_two_crossed_dials_survives() {
        // "a" dials "b" and "b" dials "a", both at once.
        let a_refuses_b = yields_to_own_dial("a", "b", true);
        let b_refuses_a = yields_to_own_dial("b", "a", true);
        assert!(a_refuses_b, "a's own dial wins, so it refuses b's");
        assert!(!b_refuses_a, "b accepts a's dial");
        assert!(!yields_to_own_dial("a", "b", false), "no crossing, nothing refused");
        assert!(refused_for_running_exchange(&format!("auth rejected: {CROSSED_DIAL_REASON}")));
        assert!(refused_for_running_exchange(&format!("auth rejected: {SESSION_ACTIVE_REASON}")));
        assert!(!refused_for_running_exchange("auth rejected: unknown device"));
    }
}
