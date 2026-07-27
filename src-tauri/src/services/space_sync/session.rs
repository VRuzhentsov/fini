//! The transport-neutral peer protocol engine: pairing gate, auth gate, and
//! the authenticated sync session loop. Operates purely on `PeerFrame` over
//! a `Link` trait object, so it is shared verbatim by every transport
//! adapter's accept/dial code (`transport::tcp_ws`, `transport::sim`, and
//! the future real Bluetooth adapter).

use std::path::PathBuf;

use diesel::prelude::*;
use tokio::sync::mpsc;

use crate::schema::{pair_space_mappings, paired_devices};
use crate::services::db::open_db_at_path;
use crate::services::device_connection::{
    DeviceConnectionState, IncomingSpaceMappingUpdate, IncomingSpaceSyncEnd, IncomingSyncAck,
};
use crate::services::space_sync::outbox::load_events_for_space;
use crate::services::space_sync::types::PeerFrame;
use crate::services::transport::{recv_frame, send_frame, Link};

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

/// Client-side auth handshake: send `Auth`, await `AuthOk`/`AuthFail`.
/// Shared by every adapter's dial path.
pub async fn perform_client_auth(
    link: &mut dyn Link,
    my_device_id: &str,
    peer_device_id: &str,
) -> Result<(), String> {
    send_frame(
        link,
        &PeerFrame::Auth {
            device_id: my_device_id.to_string(),
            peer_device_id: peer_device_id.to_string(),
        },
    )
    .await?;

    match recv_frame(link).await {
        Some(Ok(PeerFrame::AuthOk)) => Ok(()),
        Some(Ok(PeerFrame::AuthFail { reason })) => Err(format!("auth rejected: {reason}")),
        Some(Ok(_)) => Err("unexpected reply to auth".to_string()),
        Some(Err(err)) => Err(err),
        None => Err("connection closed before auth reply".to_string()),
    }
}

/// Server-side gate: read the first frame off a freshly accepted `Link` and
/// dispatch it. Pre-auth pairing messages (`PairRequest`/`PairAccept`/
/// `PairComplete`) are handled and the link is then closed — discovery and
/// pairing metadata are untrusted regardless of which transport carried
/// them. An `Auth` frame is checked against `paired_devices`; on success the
/// sticky single-session invariant is enforced via `try_claim_session`
/// before `AuthOk` is sent and the session loop starts. Transport-neutral:
/// call this from every adapter's accept loop.
///
/// `ui-plane`/`test` only: gated the same way the original `ws_server`
/// listener was — `cli-plane` dials out for sync (see the adapters'
/// `spawn_dial_loop`/`spawn_fallback_dial_loop`, ungated) but does not run
/// an inbound listener/pairing acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_peer_gate(mut link: Box<dyn Link>, state: DeviceConnectionState, db_path: PathBuf) {
    let kind = link.kind();
    let from_addr = link.peer_addr().unwrap_or_default();
    let Some(Ok(frame)) = recv_frame(link.as_mut()).await else {
        return;
    };

    let (device_id, peer_device_id) = match frame {
        PeerFrame::PairRequest(payload) => {
            let _ = state.receive_ws_pair_request(payload, from_addr);
            return;
        }
        PeerFrame::PairAccept(payload) => {
            let _ = state.receive_ws_pair_accept(payload);
            return;
        }
        PeerFrame::PairComplete(payload) => {
            let _ = state.receive_ws_pair_complete(payload);
            return;
        }
        PeerFrame::Auth {
            device_id,
            peer_device_id,
        } => (device_id, peer_device_id),
        _ => {
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

    if peer_device_id != state.identity.device_id {
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
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "unknown device".into(),
            },
        )
        .await;
        return;
    }

    let (tx, rx) = mpsc::channel::<PeerFrame>(64);
    if !state.try_claim_session(&device_id, kind, tx) {
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "session already active on another transport".into(),
            },
        )
        .await;
        return;
    }

    if send_frame(link.as_mut(), &PeerFrame::AuthOk).await.is_err() {
        state.release_session(&device_id);
        return;
    }

    run_session(link, rx, state, db_path, device_id).await;
}

/// The authenticated per-peer message loop. `rx` is the mailbox side of the
/// session sender already claimed via `DeviceConnectionState::try_claim_session`
/// (by `run_peer_gate` on accept, or the adapter's dial loop on outbound
/// connect) — this function never claims the session itself, only releases
/// it on exit.
pub async fn run_session(
    mut link: Box<dyn Link>,
    mut rx: mpsc::Receiver<PeerFrame>,
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_device_id: String,
) {
    loop {
        tokio::select! {
            inbound = recv_frame(link.as_mut()) => {
                let Some(Ok(frame)) = inbound else { break };
                handle_inbound(frame, link.as_mut(), &state, &db_path, &peer_device_id).await;
            }
            Some(frame) = rx.recv() => {
                if send_frame(link.as_mut(), &frame).await.is_err() {
                    break;
                }
            }
        }
    }

    state.release_session(&peer_device_id);
}

async fn handle_inbound(
    frame: PeerFrame,
    link: &mut dyn Link,
    state: &DeviceConnectionState,
    db_path: &PathBuf,
    peer_device_id: &str,
) {
    match frame {
        PeerFrame::SyncEvent(envelope) => {
            let event_id = envelope.event_id.clone();
            state.push_incoming_sync_event(envelope);
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
        // Sent by this side, not expected inbound
        PeerFrame::Auth { .. }
        | PeerFrame::AuthOk
        | PeerFrame::AuthFail { .. }
        | PeerFrame::PairRequest(_)
        | PeerFrame::PairAccept(_)
        | PeerFrame::PairComplete(_) => {}
    }
}
