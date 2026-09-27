//! The transport-neutral peer protocol engine: pairing gate, auth gate, and
//! the authenticated sync session loop. Operates purely on `PeerFrame` over
//! a `DataLink` trait object, so it is shared verbatim by every channel
//! adapter's accept/dial code (`channel::tcp_ws`, `channel::loopback`, and
//! the future real Bluetooth adapter).

use std::path::PathBuf;
use std::time::Duration;

use diesel::prelude::*;
use tokio::sync::mpsc;

use crate::schema::pair_space_mappings;
use crate::services::db::open_db_at_path;
use crate::services::communication::pairing::{
    channels, ChannelKind, DeviceConnectionState, IncomingSpaceMappingUpdate, IncomingSpaceSyncEnd,
    IncomingSyncAck,
};
use crate::services::communication::sync::outbox::load_events_for_space;
use crate::services::communication::sync::types::{PeerFrame, SessionCommand, PROTOCOL_VERSION};
use crate::services::communication::channel::{recv_frame, send_frame, DataLink};

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

/// How long an exchange stays open with nothing moving in either direction
/// before it closes (ADR-0008 D10). Long enough for the peer to apply what
/// it received and acknowledge it on the same connection; short enough that
/// no connection is ever held for its own sake.
pub const EXCHANGE_IDLE: Duration = Duration::from_secs(20);

/// One exchange with a peer (ADR-0008 D10): after authentication, both sides
/// push what they have, each message is acknowledged, and the connection
/// closes once nothing has moved for `idle_after` (`EXCHANGE_IDLE` in the
/// app). `rx` is the mailbox of
/// the exchange already registered with `try_claim_session`; this function
/// only releases it.
pub async fn run_session(
    mut link: Box<dyn DataLink>,
    mut rx: mpsc::Receiver<SessionCommand>,
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_device_id: String,
    peer_protocol_version: u32,
    idle_after: Duration,
) {
    let kind = link.kind();

    // Our own Bluetooth address, for the peer's diagnostics, once per
    // Network exchange -- see `PeerFrame::BluetoothAddressUpdate`.
    if link.kind() == ChannelKind::Network && peer_protocol_version >= 1 {
        if let Some(address) = crate::services::communication::pairing::local_bluetooth_address().await {
            let _ = send_frame(link.as_mut(), &PeerFrame::BluetoothAddressUpdate { address }).await;
        }
    }

    // ADR-0008 D14: every exchange restates the unlink notices this device
    // still owes the peer, until the peer acknowledges each one. Safe to
    // repeat: removing a channel that is already gone changes nothing.
    // Grows only in the app build, where a live exchange can be asked to
    // resend (`SessionCommand::SendUnlinkNotices`).
    #[cfg_attr(not(any(feature = "ui-plane", test)), allow(unused_mut))]
    let mut notices_sent = send_unlink_notices(link.as_mut(), &db_path, &peer_device_id).await;

    let idle = tokio::time::sleep(idle_after);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            inbound = recv_frame(link.as_mut()) => {
                let frame = match inbound {
                    Some(Ok(frame)) => frame,
                    Some(Err(err)) => {
                        let from = link.peer_addr().unwrap_or_else(|| "?".to_string());
                        log::info!(
                            "[exchange] {peer_device_id} {kind:?} from {from}: link error: {err}"
                        );
                        break;
                    }
                    None => {
                        log::info!("[exchange] {peer_device_id} {kind:?}: peer closed the link");
                        break;
                    }
                };
                idle.as_mut().reset(tokio::time::Instant::now() + idle_after);
                handle_inbound(frame, link.as_mut(), &state, &db_path, &peer_device_id).await;
            }
            Some(command) = rx.recv() => {
                match command {
                    SessionCommand::Forward(frame) => {
                        idle.as_mut().reset(tokio::time::Instant::now() + idle_after);
                        if send_frame(link.as_mut(), &frame).await.is_err() {
                            break;
                        }
                    }
                    #[cfg(any(feature = "ui-plane", test))]
                    SessionCommand::Close => {
                        log::info!("[exchange] {peer_device_id} {kind:?}: asked to close");
                        break;
                    }
                    #[cfg(any(feature = "ui-plane", test))]
                    SessionCommand::SendUnlinkNotices => {
                        notices_sent
                            .extend(send_unlink_notices(link.as_mut(), &db_path, &peer_device_id).await);
                    }
                }
            }
            _ = &mut idle => {
                log::info!("[exchange] {peer_device_id} {kind:?}: idle, closing");
                break;
            }
        }
    }

    state.release_session(&peer_device_id, kind);

    // A notice owed since this exchange started, that it never got to send
    // (it closed first, or its mailbox was full), is work for the next one.
    let owed = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(&db_path);
        channels::pending_unlink_notices(&mut conn, &peer_device_id)
    });
    if owed.iter().any(|kind| !notices_sent.contains(kind)) {
        crate::services::communication::sync::commands::notify_sync_work_pending();
    }
}

/// Sends every unlink notice still owed to this peer. A notice stays owed
/// until the peer's `ChannelUnlinkedAck` clears it, so a failed send simply
/// goes out again on the next exchange. Returns the ones sent.
async fn send_unlink_notices(
    link: &mut dyn DataLink,
    db_path: &PathBuf,
    peer_device_id: &str,
) -> Vec<ChannelKind> {
    let owed = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        channels::pending_unlink_notices(&mut conn, peer_device_id)
    });
    let mut sent = Vec::new();
    for kind in owed {
        if send_frame(link, &PeerFrame::ChannelUnlinked { kind }).await.is_ok() {
            sent.push(kind);
        }
    }
    sent
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
            // The same wake its two neighbours raise, and it was the only
            // one of the three without it. Arriving after this tick had
            // already drained the queue, the end would sit in memory until
            // something unrelated woke the keeper -- and since this PR
            // removed the periodic backstop, "unrelated" can mean never.
            // The peer would stay mapped on a space it had stopped sharing.
            crate::services::communication::sync::commands::notify_sync_work_pending();
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
        PeerFrame::ChannelUnlinkedAck { kind } => {
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                channels::clear_unlink_notice(&mut conn, &peer, kind);
            });
            log::info!("[session] {peer_device_id}: unlink of {kind:?} acknowledged");
        }
        // ADR-0008 D14: the peer unlinked this channel, so the pair is
        // broken for it -- remove our side too, then say so.
        PeerFrame::ChannelUnlinked { kind } => {
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            let removed = tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                channels::remove_unlinked_by_peer(&mut conn, &peer, kind)
            });
            match removed {
                Ok(()) => {
                    log::info!("[session] {peer_device_id}: peer unlinked {kind:?}; removed it here");
                    let _ = send_frame(link, &PeerFrame::ChannelUnlinkedAck { kind }).await;
                    // Whether this device should still advertise, and which
                    // channel carries waiting work, may have changed.
                    crate::services::communication::sync::commands::notify_sync_work_pending();
                }
                Err(err) => {
                    log::warn!("[session] {peer_device_id}: could not remove unlinked {kind:?}: {err}");
                }
            }
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
        | PeerFrame::Hello { .. }
        | PeerFrame::HelloAck { .. }
        // A tag this build doesn't recognize -- see `PeerFrame::Unknown`'s
        // doc comment. Ignoring it is the whole point: the session must
        // keep running rather than treat it as a decode failure.
        | PeerFrame::Unknown => {}
    }
}
