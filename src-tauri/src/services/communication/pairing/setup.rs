//! Setting a channel up for an already-paired device (ADR-0008 D1-D4).
//!
//! Both people open the setup dialog, and each device runs a setup search:
//! it says hello to the peer on that channel and answers the peer's hello
//! (the gate does that, while this search runs and afterwards while the
//! channel exists -- ADR-0008 D2). The channel's
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
#[cfg(not(test))]
const SETUP_ROUND: Duration = Duration::from_secs(60);
#[cfg(test)]
const SETUP_ROUND: Duration = Duration::from_millis(200);

/// Pause between Network hello attempts while the peer is not answering.
/// The Network channel's own presence beacon, not this, is what finds the
/// peer; this only bounds how often an unanswered hello is repeated.
const NETWORK_HELLO_RETRY: Duration = Duration::from_secs(2);

/// Pause after a Bluetooth round fails outright (scan refused, adapter
/// gone), so a broken radio is not hammered.
#[cfg(not(test))]
const BLUETOOTH_ROUND_FAILURE_PAUSE: Duration = Duration::from_secs(5);
#[cfg(test)]
const BLUETOOTH_ROUND_FAILURE_PAUSE: Duration = Duration::from_millis(200);

/// Start (or keep) the setup search for this peer's channel. The search
/// runs until `finish` ends it.
pub fn start(state: &DeviceConnectionState, peer_device_id: &str, kind: ChannelKind) {
    let already_running = state.channel_setup(peer_device_id, kind).is_some();
    let attempt = state.begin_channel_setup(peer_device_id, kind);
    // The peer must be able to reach this device while it searches, so a
    // setup is a reason to advertise (ADR-0008 D8); the keeper applies it.
    crate::services::communication::sync::commands::notify_sync_work_pending();
    if already_running {
        return;
    }
    let state = state.clone();
    let peer_device_id = peer_device_id.to_string();
    tauri::async_runtime::spawn(async move { run(state, peer_device_id, kind, attempt).await });
}

/// End the setup search. If the init completed, the channel is created
/// `On` when the person pressed OK and `Off` when they closed the dialog
/// without it (ADR-0008 D15); otherwise nothing is written. Closing never
/// switches off a channel that already exists -- the code fallback may have
/// set it up `On` while the automatic search ran.
pub fn finish(
    conn: &mut SqliteConnection,
    state: &DeviceConnectionState,
    peer_device_id: &str,
    kind: ChannelKind,
    switch_on: bool,
) -> Result<Option<ChannelSetup>, String> {
    // Ended only once the channel is written: if writing fails, the init
    // survives for another try instead of costing both people a new one.
    let setup = state.channel_setup(peer_device_id, kind);
    if setup.is_some_and(|setup| setup.initialized())
        && (switch_on || channels::find(conn, peer_device_id, kind).is_none())
    {
        channels::configure(conn, peer_device_id, kind, switch_on, None)?;
    }
    state.end_channel_setup(peer_device_id, kind);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if kind == ChannelKind::Bluetooth && setup.is_some_and(|setup| setup.initialized()) {
        state.bluetooth_advertiser.keep_answering_after_setup();
    }
    crate::services::communication::sync::commands::notify_sync_work_pending();
    Ok(setup)
}

/// Says hello until the peer acknowledges it or this attempt ends.
async fn run(state: DeviceConnectionState, peer_device_id: String, kind: ChannelKind, attempt: u64) {
    // The dialog also runs add mode for a known device, and its candidate
    // scan re-dials the very phone this search is saying hello to. Crossing
    // dials fail each other's GATT setup, so the candidate scan stands aside
    // for the whole setup -- not only until our own hello is acknowledged:
    // the peer's hello still has to reach this device afterwards, and this
    // device's scan dialling it in the meantime keeps that from happening.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let _clear_of_candidate_scan =
        (kind == ChannelKind::Bluetooth).then(|| state.bluetooth_radio.begin_pairing_leg());
    say_hello(&state, &peer_device_id, kind, attempt).await;
    if kind == ChannelKind::Bluetooth {
        while state
            .channel_setup(&peer_device_id, kind)
            .is_some_and(|setup| setup.attempt == attempt)
        {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn say_hello(state: &DeviceConnectionState, peer_device_id: &str, kind: ChannelKind, attempt: u64) {
    let (state, peer_device_id) = (state.clone(), peer_device_id.to_string());
    loop {
        match state.channel_setup(&peer_device_id, kind) {
            Some(setup) if setup.attempt == attempt && !setup.hello_acked_by_peer => {}
            _ => return,
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
            state.note_channel_setup_attempt(&peer_device_id, kind, attempt, |setup| {
                setup.hello_acked_by_peer = true
            });
            return;
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
async fn bluetooth_round(state: &DeviceConnectionState, peer_device_id: &str) -> bool {
    match crate::services::communication::channel::bluetooth::setup_hello_round(
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
    let pinned = tokio::task::block_in_place(|| {
        super::pinned_key(&mut crate::services::db::open_db_at_path(&state.db_path), peer_device_id)
    });
    let Some(peer_key) = pinned else {
        // ADR-0009 D8: a pair from before keys existed is paired again.
        return false;
    };
    let Ok(mut link) =
        crate::services::communication::channel::network::dial(state, &peer_key, ip, port).await
    else {
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
