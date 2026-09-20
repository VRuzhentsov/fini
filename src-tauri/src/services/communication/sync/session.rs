//! The transport-neutral peer protocol engine: pairing gate, auth gate, and
//! the authenticated sync session loop. Operates purely on `PeerFrame` over
//! a `DataLink` trait object, so it is shared verbatim by every channel
//! adapter's accept/dial code (`channel::tcp_ws`, `channel::sim`, and
//! the future real Bluetooth adapter).

use std::path::PathBuf;
use std::time::Duration;

use diesel::prelude::*;
use tokio::sync::mpsc;

use crate::schema::{pair_space_mappings, paired_devices};
use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::{
    channels, ChannelKind, DeviceConnectionState, IncomingSpaceMappingUpdate, IncomingSpaceSyncEnd,
    IncomingSyncAck,
};
use crate::services::communication::sync::outbox::load_events_for_space;
use crate::services::communication::sync::types::{PeerFrame, SessionCommand, PROTOCOL_VERSION};
use crate::services::communication::channel::{recv_frame, send_frame, DataLink, TransportKind};

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

/// Whether this device set this pair's Bluetooth channel up and then
/// switched it off -- checked by `BluetoothProbe`'s pre-auth handler so an
/// explicit switch-off isn't bypassed by "Find via Bluetooth". That flow's
/// whole point is discovering an address for a pair that has *never* had
/// Bluetooth set up (see its own doc comment), but a pair the person
/// actively turned off is a different case entirely. Replying would let the
/// other side believe discovery succeeded and record the address on its own
/// end, only for every real session attempt to then be rejected by
/// `check_channel_enabled` here.
///
/// Fails open, the opposite of `check_channel_enabled`: a pair with no
/// Bluetooth row has nothing to have been switched off, which is exactly
/// the never-set-up case this flow is for.
fn check_bluetooth_switched_off(db_path: &PathBuf, device_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        channels::is_switched_off(&mut conn, device_id, ChannelKind::Bluetooth)
    })
}

// ADR-0006 deleted `check_bluetooth_bond` from here. It required the
// connecting link's observed address to equal this pair's stored address and
// that address to be OS-bonded right now. Both halves are incompatible with
// how Bluetooth actually works for us: a peer advertises under a rotating
// resolvable private address, so the observed address is expected to differ
// from anything stored, and the bond that would let the OS resolve one to
// the other is something the product never creates.
//
// What it was reaching for -- "is the thing that just connected really this
// peer" -- is answered one layer up by the `Auth` frame, which
// `specs/device-connect/README.md` already names as the trust boundary.

/// Client-side auth handshake: send `Auth`, await `AuthOk`/`AuthFail`.
/// Shared by every adapter's dial path. Returns the peer's reported
/// `PROTOCOL_VERSION` (`0` for a peer running a build from before that
/// field existed) so the caller's `run_session` knows which proactive
/// frames are safe to send -- see `PROTOCOL_VERSION`'s doc comment.
pub async fn perform_client_auth(
    link: &mut dyn DataLink,
    my_device_id: &str,
    peer_device_id: &str,
) -> Result<u32, String> {
    send_frame(
        link,
        &PeerFrame::Auth {
            device_id: my_device_id.to_string(),
            peer_device_id: peer_device_id.to_string(),
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await?;

    match recv_frame(link).await {
        Some(Ok(PeerFrame::AuthOk { protocol_version })) => Ok(protocol_version),
        Some(Ok(PeerFrame::AuthFail { reason })) => Err(format!("auth rejected: {reason}")),
        Some(Ok(_)) => Err("unexpected reply to auth".to_string()),
        Some(Err(err)) => Err(err),
        None => Err("connection closed before auth reply".to_string()),
    }
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
            let _ = state.receive_ws_pair_request(payload, from_addr, kind == TransportKind::Bluetooth);
            return;
        }
        PeerFrame::PairAccept(payload) => {
            let _ = state.receive_ws_pair_accept(payload);
            return;
        }
        PeerFrame::PairComplete(payload) => {
            let _ =
                state.receive_ws_pair_complete(payload, from_addr, kind == TransportKind::Bluetooth);
            return;
        }
        PeerFrame::BluetoothProbe { device_id } => {
            // Deliberately `check_paired`, not `check_channel_enabled`:
            // this exists precisely so "Find via Bluetooth" can confirm an
            // address for a pair that doesn't have Bluetooth enabled yet.
            // But an *explicit* disable is a different case from
            // never-enabled -- see `check_bluetooth_switched_off`'s
            // doc comment for why that one must still gate the reply.
            if check_paired(&db_path, &device_id)
                && !check_bluetooth_switched_off(&db_path, &device_id)
            {
                let _ = send_frame(
                    link.as_mut(),
                    &PeerFrame::BluetoothProbeReply {
                        device_id: state.identity.device_id.clone(),
                    },
                )
                .await;
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
        _ => {
            log::warn!("[space_sync][gate] {kind:?} link from {from_addr}: expected auth first, got a different frame");
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
    if kind == TransportKind::TcpWs && !check_channel_enabled(&db_path, &device_id, ChannelKind::Network) {
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

    if kind == TransportKind::Bluetooth {
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

    let (tx, rx) = mpsc::channel::<SessionCommand>(64);
    if !state.try_claim_session(&device_id, kind, tx, &db_path) {
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
                reason: "session already active on this channel".into(),
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
        state.release_session(&device_id, kind, &db_path);
        return;
    }

    log::info!("[space_sync][gate] {kind:?} auth OK for {device_id}; starting session");
    run_session(link, rx, state, db_path, device_id, peer_protocol_version).await;
}

/// The authenticated per-peer message loop. `rx` is the mailbox side of the
/// session sender already claimed via `DeviceConnectionState::try_claim_session`
/// (by `run_peer_gate` on accept, or the adapter's dial loop on outbound
/// connect) — this function never claims the session itself, only releases
/// it on exit.
pub async fn run_session(
    mut link: Box<dyn DataLink>,
    mut rx: mpsc::Receiver<SessionCommand>,
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_device_id: String,
    peer_protocol_version: u32,
) {
    let kind = link.kind();

    // ADR-0003 revision: app-level bidirectional liveness proof. Every
    // connected channel exchanges `Ping`/`Pong` on its own, independent
    // of whether it's primary -- green must be earned per channel, not
    // borrowed from whichever one happens to carry real traffic. Gated on
    // protocol version like the Bluetooth self-report below: a peer that
    // can't decode `Ping` is left alone rather than force-closed -- its
    // channels simply stay amber forever (`AwaitingFirstAck`), a
    // degraded but correct outcome for a pre-upgrade peer.
    let ping_enabled = peer_protocol_version >= crate::services::communication::sync::types::PING_MIN_PROTOCOL_VERSION;
    let mut ping_interval = tokio::time::interval(APP_PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Self-report our own Bluetooth address once per network session, if
    // this platform can read one at all -- see `PeerFrame::BluetoothAddressUpdate`'s
    // doc comment. Only over the network channel: sending it over an
    // already-live Bluetooth session would be reporting an address the
    // other side already used to reach us. Gated on `peer_protocol_version`
    // (learned during the Auth/AuthOk exchange, see `PROTOCOL_VERSION`):
    // a peer on a build from before this frame existed cannot decode it and
    // would drop the whole authenticated session, so this frame is never
    // sent proactively to a peer that hasn't proven it understands it.
    let bluetooth_self_report_enabled = link.kind() == TransportKind::TcpWs && peer_protocol_version >= 1;
    let mut last_reported_bluetooth_address: Option<String> = None;
    if bluetooth_self_report_enabled {
        if let Some(address) = crate::services::communication::pairing::local_bluetooth_address().await {
            if send_frame(
                link.as_mut(),
                &PeerFrame::BluetoothAddressUpdate { address: address.clone() },
            )
            .await
            .is_ok()
            {
                last_reported_bluetooth_address = Some(address);
            }
        }
    }

    // Re-checked periodically, not just once at session start: a network
    // session can stay live for a long time, and if the local Bluetooth
    // controller changes underneath it (e.g. a USB dongle swap) with
    // nothing to notice, the peer is left holding a stale address with no
    // other way to refresh it -- this self-report only ever travels over
    // the network channel, so once network sync eventually breaks, the
    // Bluetooth fallback would be stuck dialing an address that no longer
    // exists. `set_missed_tick_behavior(Delay)`: a slow tick (e.g. this
    // process suspended) should never fire a burst of catch-up sends.
    let mut bluetooth_recheck = tokio::time::interval(bluetooth_recheck_interval());
    bluetooth_recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    bluetooth_recheck.tick().await; // interval fires immediately on the first tick; consume it

    loop {
        tokio::select! {
            inbound = recv_frame(link.as_mut()) => {
                let Some(Ok(frame)) = inbound else { break };
                handle_inbound(frame, link.as_mut(), &state, &db_path, &peer_device_id).await;
            }
            Some(command) = rx.recv() => {
                match command {
                    SessionCommand::Forward(frame) => {
                        if send_frame(link.as_mut(), &frame).await.is_err() {
                            break;
                        }
                    }
                    // Deliberately no frame sent to the peer here -- unlike
                    // the old Phase-3 SwitchTransport-driven close, there's
                    // no negotiated handoff for the peer to expect; this is
                    // just this device unilaterally deciding a channel it
                    // no longer wants to use (Bluetooth disabled for the
                    // pair) should stop being live. See `close_session_on`.
                    SessionCommand::Close => break,
                }
            }
            _ = bluetooth_recheck.tick(), if bluetooth_self_report_enabled => {
                if let Some(address) = crate::services::communication::pairing::local_bluetooth_address().await {
                    if last_reported_bluetooth_address.as_deref() != Some(address.as_str())
                        && send_frame(
                            link.as_mut(),
                            &PeerFrame::BluetoothAddressUpdate { address: address.clone() },
                        )
                        .await
                        .is_ok()
                    {
                        last_reported_bluetooth_address = Some(address);
                    }
                }
            }
            _ = ping_interval.tick(), if ping_enabled => {
                state.note_ping_tick(&peer_device_id, kind);
                if send_frame(link.as_mut(), &PeerFrame::Ping).await.is_err() {
                    break;
                }
            }
        }
    }

    state.release_session(&peer_device_id, kind, &db_path);
}

/// ADR-0003 revision: the app-level ping/ack cadence -- see
/// `PeerFrame::Ping`'s doc comment and `ChannelAckState`'s 3-miss decay
/// rule. Deliberately the same interval `tcp_ws::TcpWsDataLink`'s own
/// WebSocket-native ping already uses: this is a separate, channel-
/// agnostic layer on top (it also runs over Bluetooth, which has no
/// WS-level ping of its own), not a replacement for it, but there's no
/// reason for the two cadences to disagree.
///
/// Issue #171 moved it from 15s to 2 minutes. At 15s this was ~5,760 wakeups
/// a day per connected channel, in both directions, on a battery -- and
/// almost all of them proved something the channel already knew.
///
/// What makes the slower cadence safe is that a *dropped link* was never
/// detected here: `run_session`'s loop above breaks the moment its receive
/// path errors or the peer closes, and calls `release_session`, which raises
/// `LinkEvent::SessionEnded`. That is the radio telling us, and it is both
/// faster and more trustworthy than counting missed pings.
///
/// What is left for the ping is the case the channel cannot see: a peer
/// whose link is up but whose app has stopped answering. Minutes is the
/// right order for that -- nobody is served by learning it 15 seconds
/// sooner, and the row's amber state is not a thing users act on.
///
/// The cost is that `ChannelAckState`'s 3-miss decay to `PingMissed` now
/// takes ~6 minutes instead of ~45s. That only governs a live-but-wedged
/// peer; every ordinary disconnect still turns the row over immediately.
const APP_PING_INTERVAL: Duration = Duration::from_secs(120);

/// Test/CI escape hatch, mirroring `local_bluetooth_address`'s own
/// `FINI_LOCAL_BLUETOOTH_ADDRESS`: exercising the periodic re-check
/// deterministically can't wait on the real 5-minute interval.
fn bluetooth_recheck_interval() -> Duration {
    if let Ok(value) = std::env::var("FINI_BLUETOOTH_RECHECK_INTERVAL_MS") {
        if let Ok(ms) = value.parse::<u64>() {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(300)
}

async fn handle_inbound(
    frame: PeerFrame,
    link: &mut dyn DataLink,
    state: &DeviceConnectionState,
    db_path: &PathBuf,
    peer_device_id: &str,
) {
    match frame {
        PeerFrame::SyncEvent(envelope) => {
            let event_id = envelope.event_id.clone();
            state.push_incoming_sync_event(envelope);
            // Queuing it is not enough. Nothing applies an incoming event
            // until a tick drains the queue, so without this a peer's edit
            // waits for the backstop interval -- which ADR-0007 raised to 30s
            // while describing remote changes as pushed. Pushed to the queue,
            // yes; to the database and the UI, only on the next tick.
            crate::services::communication::sync::commands::notify_sync_work_pending();
            let _ = send_frame(link, &PeerFrame::Ack { event_id }).await;
        }
        PeerFrame::Ack { event_id } => {
            let acked_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            state.push_incoming_sync_ack(IncomingSyncAck {
                from_device_id: peer_device_id.to_string(),
                event_id,
                acked_at,
            });
        }
        PeerFrame::SpaceMappingUpdate {
            mapped_space_ids,
            custom_spaces,
            sent_at,
        } => {
            state.push_incoming_space_mapping_update(IncomingSpaceMappingUpdate {
                from_device_id: peer_device_id.to_string(),
                mapped_space_ids,
                custom_spaces,
                sent_at,
            });
            // Same reason as `SyncEvent` above: a mapping request the other
            // person is waiting on should not sit in a queue for a tick
            // interval before it can even be shown.
            crate::services::communication::sync::commands::notify_sync_work_pending();
            // ...and a mapping update is consumed by the *frontend*, not by
            // the tick, so waking the keeper alone would not surface it.
            // `space-sync://changed` is what `startSyncChangedListener`
            // already calls `consumeSpaceMappingUpdates` on; it simply never
            // fired for this case, because it is raised only when a tick
            // applies sync events and a mapping update produces none.
            crate::services::communication::sync::commands::note_data_changed();
        }
        PeerFrame::SpaceSyncEnd { space_id, ended_at } => {
            state.push_incoming_space_sync_end(IncomingSpaceSyncEnd {
                from_device_id: peer_device_id.to_string(),
                space_id,
                ended_at,
            });
        }
        PeerFrame::BootstrapStart { space_id } => {
            let db = db_path.clone();
            let sid = space_id.clone();
            let events = tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                load_events_for_space(&mut conn, &sid).unwrap_or_default()
            });
            for event in events {
                if send_frame(link, &PeerFrame::SyncEvent(event)).await.is_err() {
                    return;
                }
            }
            let _ = send_frame(
                link,
                &PeerFrame::BootstrapEnd {
                    space_id,
                    completed_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                },
            )
            .await;
        }
        PeerFrame::BootstrapEnd {
            space_id,
            completed_at,
        } => {
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                let _ = diesel::update(
                    pair_space_mappings::table
                        .filter(pair_space_mappings::peer_device_id.eq(&peer))
                        .filter(pair_space_mappings::space_id.eq(&space_id)),
                )
                .set(pair_space_mappings::last_synced_at.eq(Some(completed_at)))
                .execute(&mut conn);
            });
        }
        PeerFrame::Ping => {
            state.note_ping_received(peer_device_id, link.kind());
            let _ = send_frame(link, &PeerFrame::Pong).await;
        }
        PeerFrame::Pong => {
            state.note_pong_received(peer_device_id, link.kind());
        }
        PeerFrame::BluetoothAddressUpdate { address } => {
            let Some(address) = crate::services::communication::pairing::normalize_bluetooth_address(&address)
            else {
                return;
            };
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                // Real-device evidence (2026-09-01): this write can land
                // mid-lock and fail with "database is locked" -- observed
                // right after a fresh auth, when other per-tick DB activity
                // (session claim/liveness bookkeeping) is also contending
                // for the connection. `persist_bluetooth_address_and_maybe_
                // enable` is idempotent (re-applying the same address/
                // enabled state is harmless), so a short bounded retry is
                // safe and matches `try_open_db_at_path`'s own established
                // "retry on database is locked/busy" pattern for exactly
                // this class of transient SQLite contention.
                let mut last_err = String::new();
                for attempt in 0..3 {
                    if attempt > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
                    }
                    match crate::services::communication::pairing::persist_bluetooth_address_and_maybe_enable(
                        &mut conn, &peer, &address,
                    ) {
                        Ok(_) => return,
                        Err(err) => {
                            let retriable = err.contains("database is locked") || err.contains("database is busy");
                            last_err = err;
                            if !retriable {
                                break;
                            }
                        }
                    }
                }
                eprintln!("[space-sync] persist bluetooth self-report failed: {last_err}");
            });
        }
        // Pre-auth only (handled earlier in run_peer_gate's first-frame
        // dispatch) or sent by this side -- never expected inbound here.
        PeerFrame::Auth { .. }
        | PeerFrame::AuthOk { .. }
        | PeerFrame::AuthFail { .. }
        | PeerFrame::PairRequest(_)
        | PeerFrame::PairAccept(_)
        | PeerFrame::PairComplete(_)
        | PeerFrame::DiscoveryHello
        | PeerFrame::DiscoveryHelloReply { .. }
        | PeerFrame::BluetoothProbe { .. }
        | PeerFrame::BluetoothProbeReply { .. }
        // A tag this build doesn't recognize -- see `PeerFrame::Unknown`'s
        // doc comment. Ignoring it is the whole point: the session must
        // keep running rather than treat it as a decode failure.
        | PeerFrame::Unknown => {}
    }
}
