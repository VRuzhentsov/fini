use std::path::PathBuf;

use diesel::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

use crate::services::db::open_db_at_path;
use crate::services::device_connection::DeviceConnectionState;
use crate::services::space_sync::types::WsMessage;
use crate::services::space_sync::ws_session;

pub async fn run_ws_server(state: DeviceConnectionState, db_path: PathBuf) {
    let port = state.space_sync_ws_port;
    run_ws_server_on_port(state, db_path, port).await;
}

pub(crate) async fn run_ws_server_on_port(
    state: DeviceConnectionState,
    db_path: PathBuf,
    port: u16,
) {
    let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[WS][SERVER] Failed to bind :{}: {}", port, e);
            return;
        }
    };
    eprintln!("[WS][SERVER] Listening on :{}", port);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!("[WS][SERVER] Connection from {}", addr);
                let state = state.clone();
                let db_path = db_path.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, state, db_path).await {
                        eprintln!("[WS][SERVER] Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("[WS][SERVER] Accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: DeviceConnectionState,
    db_path: PathBuf,
) -> Result<(), String> {
    let peer_ip = stream
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_default();
    let ws = accept_async(stream)
        .await
        .map_err(|e| format!("WS handshake: {}", e))?;
    let (mut sink, mut source) = ws.split();

    let Some(Ok(raw)) = source.next().await else {
        return Ok(());
    };
    let Message::Text(text) = raw else {
        return Ok(());
    };
    let Ok(msg) = serde_json::from_str::<WsMessage>(&text) else {
        return Ok(());
    };

    let (device_id, peer_device_id) = match msg {
        WsMessage::PairRequest(payload) => {
            state.receive_ws_pair_request(payload, peer_ip)?;
            return Ok(());
        }
        WsMessage::PairAccept(payload) => {
            state.receive_ws_pair_accept(payload)?;
            return Ok(());
        }
        WsMessage::PairComplete(payload) => {
            state.receive_ws_pair_complete(payload)?;
            return Ok(());
        }
        WsMessage::Auth {
            device_id,
            peer_device_id,
        } => (device_id, peer_device_id),
        _ => {
            send_msg(
                &mut sink,
                &WsMessage::AuthFail {
                    reason: "expected auth first".into(),
                },
            )
            .await;
            return Ok(());
        }
    };

    if peer_device_id != state.identity.device_id {
        send_msg(
            &mut sink,
            &WsMessage::AuthFail {
                reason: "wrong target device".into(),
            },
        )
        .await;
        return Ok(());
    }

    let paired = check_paired(&db_path, &device_id);
    if !paired {
        send_msg(
            &mut sink,
            &WsMessage::AuthFail {
                reason: "unknown device".into(),
            },
        )
        .await;
        return Ok(());
    }

    send_msg(&mut sink, &WsMessage::AuthOk).await;
    ws_session::run_session(sink, source, state, db_path, device_id).await;
    Ok(())
}

fn check_paired(db_path: &PathBuf, device_id: &str) -> bool {
    use crate::schema::paired_devices;
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

async fn send_msg<Sk>(sink: &mut Sk, msg: &WsMessage)
where
    Sk: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    if let Ok(text) = serde_json::to_string(msg) {
        let _ = sink.send(Message::Text(text.into())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreatePairedDeviceInput;
    use crate::schema::paired_devices;
    use crate::services::db::{open_db_at_path, temp_db_path};
    use crate::services::device_connection::DeviceConnectionState;
    use crate::services::space_sync::types::WsMessage;
    use futures_util::{SinkExt, StreamExt};
    use std::path::PathBuf;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    struct TestServer {
        port: u16,
        state: DeviceConnectionState,
    }

    impl TestServer {
        async fn start(db_path: PathBuf) -> Self {
            // Use a random port
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);

            let data_dir = db_path.with_extension("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            let state = DeviceConnectionState::new(&data_dir);
            let state_clone = state.clone();
            let db_clone = db_path.clone();

            tokio::spawn(async move {
                run_ws_server_on_port(state_clone, db_clone, port).await;
            });
            // Give server a moment to bind
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            Self { port, state }
        }

        async fn connect_and_recv_first(&self, auth_msg: WsMessage) -> WsMessage {
            let url = format!("ws://127.0.0.1:{}", self.port);
            let (mut ws, _) = connect_async(&url).await.expect("connect");
            let text = serde_json::to_string(&auth_msg).unwrap();
            ws.send(Message::Text(text.into())).await.unwrap();
            let raw = ws.next().await.unwrap().unwrap();
            let Message::Text(t) = raw else {
                panic!("not text")
            };
            serde_json::from_str(&t).unwrap()
        }
    }

    fn seed_paired(db_path: &PathBuf, device_id: &str) {
        let mut conn = open_db_at_path(db_path);
        diesel::insert_into(paired_devices::table)
            .values(&CreatePairedDeviceInput {
                peer_device_id: device_id.to_string(),
                display_name: "Test Peer".to_string(),
                paired_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .execute(&mut conn)
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_auth_rejects_unknown_device_id() {
        let db_path = temp_db_path("ws-auth-unknown");
        let srv = TestServer::start(db_path).await;

        let reply = srv
            .connect_and_recv_first(WsMessage::Auth {
                device_id: "not-paired".to_string(),
                peer_device_id: srv.state.identity.device_id.clone(),
            })
            .await;

        assert!(
            matches!(reply, WsMessage::AuthFail { .. }),
            "expected AuthFail, got {:?}",
            reply
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_auth_accepts_paired_device() {
        let db_path = temp_db_path("ws-auth-paired");
        seed_paired(&db_path, "peer-abc");

        let srv = TestServer::start(db_path).await;

        let reply = srv
            .connect_and_recv_first(WsMessage::Auth {
                device_id: "peer-abc".to_string(),
                peer_device_id: srv.state.identity.device_id.clone(),
            })
            .await;

        assert!(
            matches!(reply, WsMessage::AuthOk),
            "expected AuthOk, got {:?}",
            reply
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_sync_event_round_trip() {
        use crate::services::space_sync::types::SyncEventEnvelope;

        let db_path = temp_db_path("ws-sync-event-rt");
        seed_paired(&db_path, "peer-rt");

        let srv = TestServer::start(db_path).await;
        let url = format!("ws://127.0.0.1:{}", srv.port);
        let (mut ws, _) = connect_async(&url).await.unwrap();

        // Auth
        let auth = serde_json::to_string(&WsMessage::Auth {
            device_id: "peer-rt".to_string(),
            peer_device_id: srv.state.identity.device_id.clone(),
        })
        .unwrap();
        ws.send(Message::Text(auth.into())).await.unwrap();
        ws.next().await.unwrap().unwrap(); // AuthOk

        // Send a SyncEvent
        let envelope = SyncEventEnvelope {
            event_id: "evt-1".to_string(),
            correlation_id: "cor-1".to_string(),
            origin_device_id: "peer-rt".to_string(),
            entity_type: "quest".to_string(),
            entity_id: "q1".to_string(),
            space_id: "1".to_string(),
            op_type: "upsert".to_string(),
            payload: Some(r#"{"title":"Test"}"#.to_string()),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let event_text = serde_json::to_string(&WsMessage::SyncEvent(envelope)).unwrap();
        ws.send(Message::Text(event_text.into())).await.unwrap();

        // Expect Ack
        let raw = ws.next().await.unwrap().unwrap();
        let Message::Text(t) = raw else {
            panic!("not text")
        };
        let reply: WsMessage = serde_json::from_str(&t).unwrap();
        assert!(
            matches!(reply, WsMessage::Ack { ref event_id } if event_id == "evt-1"),
            "expected Ack for evt-1, got {:?}",
            reply
        );

        // Event must be in state's incoming queue
        let events = srv.state.take_incoming_sync_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "evt-1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_bootstrap_streams_space_events() {
        use crate::services::space_sync::outbox::emit_sync_event;

        let db_path = temp_db_path("ws-bootstrap");
        seed_paired(&db_path, "peer-bs");

        // Seed outbox with 2 events for space "1" and 1 for space "2"
        {
            let mut conn = open_db_at_path(&db_path);
            emit_sync_event(&mut conn, "origin", "quest", "q1", "1", "upsert", None).unwrap();
            emit_sync_event(&mut conn, "origin", "quest", "q2", "1", "upsert", None).unwrap();
            emit_sync_event(&mut conn, "origin", "quest", "q3", "2", "upsert", None).unwrap();
        }

        let srv = TestServer::start(db_path).await;
        let url = format!("ws://127.0.0.1:{}", srv.port);
        let (mut ws, _) = connect_async(&url).await.unwrap();

        let auth = serde_json::to_string(&WsMessage::Auth {
            device_id: "peer-bs".to_string(),
            peer_device_id: srv.state.identity.device_id.clone(),
        })
        .unwrap();
        ws.send(Message::Text(auth.into())).await.unwrap();
        ws.next().await.unwrap().unwrap(); // AuthOk

        let bs_start = serde_json::to_string(&WsMessage::BootstrapStart {
            space_id: "1".to_string(),
        })
        .unwrap();
        ws.send(Message::Text(bs_start.into())).await.unwrap();

        let mut sync_events = 0usize;
        let mut got_end = false;
        for _ in 0..10 {
            let raw = ws.next().await.unwrap().unwrap();
            let Message::Text(t) = raw else { break };
            let msg: WsMessage = serde_json::from_str(&t).unwrap();
            match msg {
                WsMessage::SyncEvent(_) => sync_events += 1,
                WsMessage::BootstrapEnd {
                    space_id,
                    completed_at: _,
                } => {
                    assert_eq!(space_id, "1");
                    got_end = true;
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(sync_events, 2, "expected 2 events for space 1");
        assert!(got_end, "expected BootstrapEnd");
    }
}
