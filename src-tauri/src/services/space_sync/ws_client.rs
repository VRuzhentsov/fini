use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::services::device_connection::{DeviceConnectionState, SPACE_SYNC_WS_PORT};
use crate::services::space_sync::types::WsMessage;
use crate::services::space_sync::ws_session;

/// Call from `space_sync_tick`: ensure an outbound session exists for every
/// presenced peer where `self.device_id < peer.device_id` (deterministic dialer rule).
pub fn ensure_peer_sessions(state: &DeviceConnectionState, db_path: PathBuf) {
    let my_id = state.identity.device_id.clone();
    let peers = state.list_presenced_peers();

    for (peer_id, addr) in peers {
        if my_id >= peer_id {
            continue; // The peer is the dialer for this pair
        }
        if state.has_session(&peer_id) {
            continue; // Already connected
        }
        let state = state.clone();
        let db_path = db_path.clone();
        let peer_id_clone = peer_id.clone();
        tokio::spawn(async move {
            dial_with_backoff(state, db_path, peer_id_clone, addr).await;
        });
    }
}

async fn dial_with_backoff(
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_id: String,
    addr: String,
) {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);

    loop {
        // Stop if the session appeared (e.g. peer dialed us instead)
        if state.has_session(&peer_id) {
            return;
        }
        // Stop if peer is no longer presenced (gone offline)
        let still_present = state
            .list_presenced_peers()
            .into_iter()
            .any(|(id, _)| id == peer_id);
        if !still_present {
            return;
        }

        let url = format!("ws://{}:{}", addr, SPACE_SYNC_WS_PORT);
        match connect_async(&url).await {
            Ok((ws, _)) => {
                let (mut sink, mut source) = ws.split();
                let auth = WsMessage::Auth {
                    device_id: state.identity.device_id.clone(),
                    peer_device_id: peer_id.clone(),
                };
                let Ok(text) = serde_json::to_string(&auth) else { return };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(max_delay);
                    continue;
                }
                // Read auth response
                let Some(Ok(raw)) = source.next().await else {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(max_delay);
                    continue;
                };
                let Message::Text(t) = raw else {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(max_delay);
                    continue;
                };
                match serde_json::from_str::<WsMessage>(&t) {
                    Ok(WsMessage::AuthOk) => {
                        eprintln!("[WS][CLIENT] Auth OK with {}", peer_id);
                        // session loop blocks until disconnect
                        ws_session::run_session(sink, source, state.clone(), db_path.clone(), peer_id.clone()).await;
                        eprintln!("[WS][CLIENT] Session with {} ended", peer_id);
                        // After disconnect, pause then retry
                        delay = Duration::from_secs(1);
                    }
                    Ok(WsMessage::AuthFail { reason }) => {
                        eprintln!("[WS][CLIENT] Auth rejected by {}: {}", peer_id, reason);
                        return; // Not paired; don't retry
                    }
                    _ => {}
                }
            }
            Err(e) => {
                eprintln!("[WS][CLIENT] Connect {} failed: {}", url, e);
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}
