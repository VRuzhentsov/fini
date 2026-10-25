//! The transport-neutral peer protocol engine: pairing gate, auth gate, and
//! the authenticated sync session loop. Operates purely on `PeerFrame` over
//! a `DataLink` trait object, so it is shared verbatim by every channel
//! adapter's accept/dial code (`channel::network`, `channel::loopback`, and
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
use crate::services::communication::sync::types::{PeerFrame, SessionCommand};
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
// peer" -- is answered by the TLS handshake against the key pinned for the
// pair (`pairing::run_session_gate`).

/// QUIC close codes of a session the peer's gate refused
/// (`pairing::run_session_gate`), which the connecting side reads as its
/// close reason. There is no refusal frame (2026-10-13: TLS only).
pub(crate) const CLOSE_NOT_PAIRED: u32 = 1;
pub(crate) const CLOSE_CHANNEL_OFF: u32 = 2;
/// Both devices connected at once; the smaller device id's connection is
/// kept.
#[cfg(any(feature = "ui-plane", test))]
pub(crate) const CLOSE_CROSSED: u32 = 3;
#[cfg(any(feature = "ui-plane", test))]
pub(crate) const CLOSE_SESSION_ACTIVE: u32 = 4;
/// The pair is there, but the peer has no such channel for it: it was
/// removed there, or never added. The same answer for both, so it says
/// nothing about the past. The connecting side removes its own row.
pub(crate) const CLOSE_CHANNEL_MISSING: u32 = 5;

/// Whether a session the peer closed with `close_code` was refused by its
/// gate as not wanted (not paired there, or the channel is off or missing
/// there): the connecting side counts that as a failed attempt and waits
/// before it tries again. A crossing or an active session is no failure: the
/// peer's own session carries the work.
pub(crate) fn refused_by_peer(close_code: Option<u32>) -> bool {
    matches!(close_code, Some(CLOSE_NOT_PAIRED | CLOSE_CHANNEL_OFF | CLOSE_CHANNEL_MISSING))
}

/// The peer has no `kind` channel for this pair any more (ADR-0008 D14):
/// remove ours too. From its `ChannelUnlinked` frame, or from its gate's
/// `CLOSE_CHANNEL_MISSING`.
pub(crate) fn remove_channel_the_peer_has_not(
    state: &DeviceConnectionState, peer_device_id: &str, kind: ChannelKind,
) {
    let removed = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(&state.db_path);
        channels::remove_unlinked_by_peer(&mut conn, peer_device_id, kind)
    });
    match removed {
        Ok(()) => {
            log::info!("[session] {peer_device_id}: peer has no {kind:?} channel; removed it here");
            // Whether this device should still advertise, and which
            // channel carries waiting work, may have changed.
            state.space_sync.wake();
        }
        Err(err) => {
            log::warn!("[session] {peer_device_id}: could not remove {kind:?}: {err}");
        }
    }
}


/// How long a session on this channel stays open with nothing moving.
/// `None`: the session is held, and closes only when its link fails -- a
/// held session is what makes the channel green (2026-10-13 decision: green
/// means a live connection). Both channels hold.
pub fn idle_after(kind: ChannelKind) -> Option<Duration> {
    match kind {
        ChannelKind::Network | ChannelKind::Bluetooth => None,
    }
}

/// How long after this device's own connection to a peer became a session a
/// connection from that peer counts as a crossing (both connected at the
/// same moment; the smaller device id's connection is kept). Later, it is a
/// peer that restarted, and replaces the session. See each channel's
/// `connecting`.
#[cfg(any(feature = "ui-plane", test))]
pub(crate) const CROSSING_WINDOW: Duration = Duration::from_secs(3);

/// Writes every frame kept for the peer to the link, dropping each once
/// written. `false` if the link failed; what was not written stays kept.
async fn send_kept_frames(link: &mut dyn DataLink, db_path: &PathBuf, peer_device_id: &str) -> bool {
    let waiting = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        super::control_outbox::waiting(&mut conn, peer_device_id)
    });
    for (id, frame) in waiting {
        if send_frame(link, &frame).await.is_err() {
            return false;
        }
        tokio::task::block_in_place(|| {
            let mut conn = open_db_at_path(db_path);
            super::control_outbox::sent(&mut conn, id);
        });
    }
    true
}

/// One exchange with a peer (ADR-0008 D10): after authentication, both sides
/// push what they have, each message is acknowledged, and the connection
/// closes once nothing has moved for `idle_after`, or only when the link
/// fails if `idle_after` is `None` (see `idle_after()`). `rx` is the mailbox
/// of the exchange already registered with `try_claim_session`; this
/// function only releases it. Returns the close code the peer closed the
/// link with, if it did (`pairing::gate::refused_by_peer`).
pub async fn run_session(
    mut link: Box<dyn DataLink>,
    mut rx: mpsc::Receiver<SessionCommand>,
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_device_id: String,
    idle_after: Option<Duration>,
) -> Option<u32> {
    let mut close_code = None;
    let closes_when_idle = idle_after.is_some();
    // Only read while `closes_when_idle`; a held session never waits on it.
    let idle_after = idle_after.unwrap_or(Duration::from_secs(3600));
    let kind = link.kind();

    // Our own Bluetooth address, for the peer's diagnostics, once per
    // Network exchange -- see `PeerFrame::BluetoothAddressUpdate`.
    if link.kind() == ChannelKind::Network {
        if let Some(address) = crate::services::communication::pairing::local_bluetooth_address().await {
            let _ = send_frame(link.as_mut(), &PeerFrame::BluetoothAddressUpdate { address }).await;
        }
    }

    // Frames kept for this peer (ADR-0008 D14) go first, and again whenever
    // more are kept while the exchange runs.
    let mut kept = super::control_outbox::changed().subscribe();
    kept.mark_changed();

    let idle = tokio::time::sleep(idle_after);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            Ok(()) = kept.changed() => {
                idle.as_mut().reset(tokio::time::Instant::now() + idle_after);
                if !send_kept_frames(link.as_mut(), &db_path, &peer_device_id).await {
                    break;
                }
            }
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
                        let why = link.close_reason().unwrap_or_default();
                        log::info!("[exchange] {peer_device_id} {kind:?}: peer closed the link {why}");
                        close_code = link.close_code();
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
                }
            }
            _ = &mut idle, if closes_when_idle => {
                log::info!("[exchange] {peer_device_id} {kind:?}: idle, closing");
                break;
            }
        }
    }

    state.release_session(&peer_device_id, kind);

    // Anything this exchange left unfinished -- an event sent but not
    // acknowledged, a bootstrap not ended -- is
    // still durable work; a tick after a pause looks again, so a dropped
    // exchange does not stall sync, and a peer that keeps dropping off does
    // not turn into back-to-back exchanges. A tick with nothing left to do
    // starts nothing.
    state.space_sync.wake_after(
        EXCHANGE_END_RECHECK,
    );
    // A held session ends only when its link failed: the channel is grey
    // now, and the next tick lets it connect again (`keep_serving`) if the
    // peer is still there.
    if !closes_when_idle {
        state.space_sync.wake_after(HELD_SESSION_RECONNECT);
    }
    close_code
}

/// How soon after a held session ends the channel tries to connect again.
const HELD_SESSION_RECONNECT: Duration = Duration::from_secs(2);

/// How long after an exchange ends the keeper looks again for work it left
/// unfinished.
const EXCHANGE_END_RECHECK: Duration = Duration::from_secs(60);

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
            state.space_sync.wake();
            let _ = send_frame(link, &PeerFrame::Ack { event_id }).await;
        }
        PeerFrame::Ack { event_id } => {
            let acked_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            state.push_incoming_sync_ack(IncomingSyncAck {
                from_device_id: peer_device_id.to_string(),
                event_id,
                acked_at,
            });
            // Only a tick clears the delivered entry from the outbox.
            state.space_sync.wake();
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
            state.space_sync.wake();
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
            state.space_sync.wake();
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
        // ADR-0008 D14: the peer unlinked this channel, so the pair is
        // broken for it -- remove our side too.
        PeerFrame::ChannelUnlinked { kind } => {
            remove_channel_the_peer_has_not(state, peer_device_id, kind);
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
        // Frames before a session (`PEER_ALPN`, handled by run_peer_gate) --
        // never expected inbound here.
        PeerFrame::PairStart(_)
        | PeerFrame::PairAccept(_)
        | PeerFrame::PairComplete(_)
        | PeerFrame::IdentifyDevice { .. }
        | PeerFrame::DeviceIdentified { .. }
        | PeerFrame::AddChannel { .. }
        | PeerFrame::AddChannelAccept { .. }
        // A tag this build doesn't recognize -- see `PeerFrame::Unknown`'s
        // doc comment. Ignoring it is the whole point: the session must
        // keep running rather than treat it as a decode failure.
        | PeerFrame::Unknown => {}
    }
}
