//! Setting a channel up for an already-paired device (ADR-0008 D1-D4).
//!
//! Both people open the setup dialog, and each device runs a setup search:
//! it says hello to the peer on that channel and answers the peer's hello
//! (the gate does that, and only while this search runs). The channel's
//! init is complete on a device once both halves happened there -- its own
//! hello was acknowledged, and it acknowledged the peer's. Only then can
//! the person press OK, which switches the channel on for this device
//! alone.

use std::time::Duration;

use diesel::SqliteConnection;

use super::{channels, ChannelKind, ChannelSetup, DeviceConnectionState};
use crate::services::communication::channel::{recv_frame, send_frame};
use crate::services::communication::sync::types::PeerFrame;

/// How long one Bluetooth setup round listens before the next one starts
/// (ADR-0008 D12: a search lasts up to 60s and ends early on success).
const SETUP_ROUND: Duration = Duration::from_secs(60);

/// Pause between Network hello attempts while the peer is not answering.
/// The Network channel's own presence beacon, not this, is what finds the
/// peer; this only bounds how often an unanswered hello is repeated.
const NETWORK_HELLO_RETRY: Duration = Duration::from_secs(2);

/// Pause after a Bluetooth round fails outright (scan refused, adapter
/// gone), so a broken radio is not hammered.
const BLUETOOTH_ROUND_FAILURE_PAUSE: Duration = Duration::from_secs(5);

/// Start (or keep) the setup search for this peer's channel. The search
/// runs until `finish` ends it.
pub fn start(state: &DeviceConnectionState, peer_device_id: &str, kind: ChannelKind) {
    let already_running = state.channel_setup(peer_device_id, kind).is_some();
    state.begin_channel_setup(peer_device_id, kind);
    // The peer must be able to reach this device while it searches, so a
    // setup is a reason to advertise (ADR-0008 D8); the keeper applies it.
    crate::services::communication::sync::commands::notify_sync_work_pending();
    if already_running {
        return;
    }
    let state = state.clone();
    let peer_device_id = peer_device_id.to_string();
    tauri::async_runtime::spawn(async move { run(state, peer_device_id, kind).await });
}

/// End the setup search. If the init completed, the channel is created
/// `On` when the person pressed OK and `Off` when they closed the dialog
/// without it (ADR-0008 D15); otherwise nothing is written.
pub fn finish(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: &str,
    kind: ChannelKind,
    switch_on: bool,
) -> Result<Option<ChannelSetup>, String> {
    let setup = state.end_channel_setup(peer_device_id, kind);
    if setup.is_some_and(|setup| setup.initialized()) {
        channels::configure(conn, peer_device_id, kind, switch_on, None)?;
    }
    crate::services::communication::sync::commands::notify_sync_work_pending();
    Ok(setup)
}

/// Says hello until the peer acknowledges it or the search ends.
async fn run(state: DeviceConnectionState, peer_device_id: String, kind: ChannelKind) {
    loop {
        match state.channel_setup(&peer_device_id, kind) {
            None => return,
            Some(setup) if setup.hello_acked_by_peer => return,
            Some(_) => {}
        }
        let acknowledged = match kind {
            ChannelKind::Bluetooth => bluetooth_round(&state, &peer_device_id).await,
            ChannelKind::Network => {
                let acknowledged = network_hello(&state, &peer_device_id).await;
                if !acknowledged {
                    tokio::time::sleep(NETWORK_HELLO_RETRY).await;
                }
                acknowledged
            }
        };
        if acknowledged {
            state.note_channel_setup(&peer_device_id, kind, |setup| setup.hello_acked_by_peer = true);
            return;
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
async fn bluetooth_round(state: &DeviceConnectionState, peer_device_id: &str) -> bool {
    match crate::services::communication::channel::ble::setup_hello_round(
        state.clone(),
        peer_device_id.to_string(),
        SETUP_ROUND,
    )
    .await
    {
        Ok(acknowledged) => acknowledged,
        Err(err) => {
            log::warn!("[setup] bluetooth round for {peer_device_id} failed: {err}");
            tokio::time::sleep(BLUETOOTH_ROUND_FAILURE_PAUSE).await;
            false
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
async fn bluetooth_round(_state: &DeviceConnectionState, _peer_device_id: &str) -> bool {
    tokio::time::sleep(BLUETOOTH_ROUND_FAILURE_PAUSE).await;
    false
}

/// One hello over the Network channel, to wherever the peer's presence
/// beacon was last heard. `false` if the peer is not present or did not
/// acknowledge.
async fn network_hello(state: &DeviceConnectionState, peer_device_id: &str) -> bool {
    let Some((addr, port)) = state.network_presence_address(peer_device_id) else {
        return false;
    };
    let Ok(ip) = addr.parse() else {
        return false;
    };
    let Ok(mut link) = crate::services::communication::channel::tcp_ws::dial(ip, port).await else {
        return false;
    };
    let hello = PeerFrame::Hello {
        device_id: state.identity.device_id.clone(),
    };
    if send_frame(link.as_mut(), &hello).await.is_err() {
        return false;
    }
    matches!(
        tokio::time::timeout(Duration::from_secs(5), recv_frame(link.as_mut())).await,
        Ok(Some(Ok(PeerFrame::HelloAck { device_id }))) if device_id == peer_device_id
    )
}
