use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use diesel::prelude::*;

use crate::schema::pair_space_mappings;
use crate::services::db::open_db_at_path;
use crate::services::device_connection::{
    DeviceConnectionState, IncomingSpaceMappingUpdate, IncomingSyncAck,
};
use crate::services::space_sync::outbox::load_events_for_space;
use crate::services::space_sync::types::WsMessage;

/// Run the bidirectional session loop after authentication.
/// Registers the session in `state` so the sync tick can enqueue outbound messages.
pub async fn run_session<Sk, Sr>(
    mut sink: Sk,
    mut source: Sr,
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_device_id: String,
) where
    Sk: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin + Send,
    Sr: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin
        + Send,
{
    let (tx, mut rx) = mpsc::channel::<WsMessage>(64);
    state.register_session(peer_device_id.clone(), tx);

    loop {
        tokio::select! {
            inbound = source.next() => {
                let Some(Ok(raw)) = inbound else { break };
                let Message::Text(text) = raw else { continue };
                let Ok(msg) = serde_json::from_str::<WsMessage>(&text) else { continue };
                handle_inbound(msg, &mut sink, &state, &db_path, &peer_device_id).await;
            }
            Some(msg) = rx.recv() => {
                let Ok(text) = serde_json::to_string(&msg) else { continue };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }

    state.unregister_session(&peer_device_id);
}

async fn handle_inbound<Sk>(
    msg: WsMessage,
    sink: &mut Sk,
    state: &DeviceConnectionState,
    db_path: &PathBuf,
    peer_device_id: &str,
) where
    Sk: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    match msg {
        WsMessage::SyncEvent(envelope) => {
            let event_id = envelope.event_id.clone();
            state.push_incoming_sync_event(envelope);
            let ack = serde_json::to_string(&WsMessage::Ack { event_id }).unwrap_or_default();
            let _ = sink.send(Message::Text(ack.into())).await;
        }
        WsMessage::Ack { event_id } => {
            let acked_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            state.push_incoming_sync_ack(IncomingSyncAck {
                from_device_id: peer_device_id.to_string(),
                event_id,
                acked_at,
            });
        }
        WsMessage::SpaceMappingUpdate {
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
        WsMessage::BootstrapStart { space_id } => {
            let db = db_path.clone();
            let sid = space_id.clone();
            let events = tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                load_events_for_space(&mut conn, &sid).unwrap_or_default()
            });
            for event in events {
                let Ok(text) = serde_json::to_string(&WsMessage::SyncEvent(event)) else {
                    continue;
                };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
            let Ok(end) = serde_json::to_string(&WsMessage::BootstrapEnd { space_id }) else {
                return;
            };
            let _ = sink.send(Message::Text(end.into())).await;
        }
        WsMessage::BootstrapEnd { space_id } => {
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                let _ = diesel::update(
                    pair_space_mappings::table
                        .filter(pair_space_mappings::peer_device_id.eq(&peer))
                        .filter(pair_space_mappings::space_id.eq(&space_id)),
                )
                .set(pair_space_mappings::last_synced_at.eq(Some(now)))
                .execute(&mut conn);
            });
        }
        // Sent by this side, not expected inbound
        WsMessage::Auth { .. } | WsMessage::AuthOk | WsMessage::AuthFail { .. } => {}
    }
}

