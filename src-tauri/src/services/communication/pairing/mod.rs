mod carrier;
mod commands;
mod device_key;
mod flow;
mod runtime;
pub(crate) mod channel_status;
pub(crate) mod channels;
#[cfg(any(feature = "ui-plane", test))]
pub(crate) mod setup;
#[cfg(any(feature = "ui-plane", test))]
pub(crate) mod gate;
pub(crate) mod types;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::services::communication::sync::types::{PeerFrame, SessionCommand, SessionSender, SyncEventEnvelope};
use crate::services::communication::channel::selection::{new_lifecycle_bus, LifecycleBus, LifecycleEvent};

// Shared with `channel::tests`, which sets/clears the same process-global
// `FINI_BLUETOOTH_PAIRED_ADDRESSES` env var in its own tests -- see the
// lock's own doc comment for why this must be one lock, not two.
#[cfg(test)]
pub(crate) use commands::BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK;

#[cfg(any(feature = "ui-plane", test))]
pub use commands::{
    device_connection_consume_space_mapping_updates, device_connection_debug_status,
    device_connection_bluetooth_available_devices, device_connection_identify_bluetooth_device,
    device_connection_discovery_snapshot,
    device_connection_enter_add_mode, device_connection_begin_channel_setup,
    device_connection_channel_setup_status, device_connection_end_channel_setup,
    device_connection_get_identity, device_connection_get_paired_devices,
    device_connection_leave_add_mode, device_connection_pair_accept_request,
    device_connection_pair_acknowledge_request, device_connection_pair_complete_request,
    device_connection_pair_incoming_requests, device_connection_pair_outgoing_completions,
    device_connection_pair_outgoing_updates, device_connection_presence_snapshot,
    device_connection_save_paired_device,
    device_connection_send_pair_request, device_connection_send_pair_request_bluetooth,
    device_connection_probe_bluetooth_adapter,
    device_connection_session_channel, device_connection_set_channel_enabled,
    device_connection_set_primary_channel,
    device_connection_unlink_channel, device_connection_watch_presence,
    device_connection_channel_statuses, device_connection_unpair, device_connection_update_last_seen,
};

#[cfg(all(feature = "ui-plane", feature = "devtools"))]
pub use commands::e2e_set_radio;

#[cfg(any(feature = "ui-plane", test))]
pub use gate::run_peer_gate;

pub use commands::bluetooth_connect_candidates;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use commands::note_observed_bluetooth_address;
// ADR-0006: `bluetooth_address_is_os_paired` is no longer re-exported. The
// bond check has no callers outside `commands` now that neither the dial
// path nor the inbound gate consults it.
pub(crate) use commands::{
    local_bluetooth_address, normalize_bluetooth_address,
    persist_bluetooth_address_and_maybe_enable,
};
#[cfg(any(feature = "cli-plane", test))]
pub use commands::{
    device_connection_consume_space_mapping_updates_impl, device_connection_debug_status_impl,
    device_connection_discovery_snapshot_impl, device_connection_enter_add_mode_impl,
    device_connection_get_identity_impl, device_connection_get_paired_devices_impl,
    device_connection_leave_add_mode_impl, device_connection_pair_accept_request_impl,
    device_connection_pair_acknowledge_request_impl, device_connection_pair_complete_request_impl,
    device_connection_pair_incoming_requests_impl,
    device_connection_pair_outgoing_completions_impl, device_connection_pair_outgoing_updates_impl,
    device_connection_presence_snapshot_impl, device_connection_save_paired_device_impl,
    device_connection_send_pair_request_impl,
    device_connection_unpair_impl, device_connection_update_last_seen_impl,
};
// Channel commands only the app offers; the app reaches them through
// `commands` directly, so only the tests need them here.
#[cfg(test)]
pub use commands::{
    device_connection_channel_statuses_impl, device_connection_set_channel_enabled_impl,
    device_connection_set_primary_channel_impl, device_connection_unlink_channel_impl,
};
// `ChannelKind` is the channel a pair configured -- Network or Bluetooth --
// and is what the `channels` table stores. Re-exported from `channel`,
// where it is defined; there is no second enum saying the same thing any
// more.
pub use channel_status::ChannelKind;
// How a channel row is drawn (ADR-0008 D19) -- the app's concern alone.
#[cfg(any(feature = "ui-plane", test))]
pub use channel_status::{channel_status, ChannelProblem, ChannelState, ChannelStatus};
use types::DiscoveryRuntime;
#[cfg(any(feature = "ui-plane", test))]
pub use types::ChannelSetup;
pub use types::{
    CustomSpaceDescriptor, DeviceIdentity, IncomingSpaceMappingUpdate, IncomingSpaceSyncEnd,
    IncomingSyncAck,
};
#[cfg(any(feature = "ui-plane", test))]
use types::{
    PairAcceptPayload, PairCodeUpdate, PairCompletePayload, PairCompletionUpdate,
    PairRequestPayload,
};

pub const HEARTBEAT_INTERVAL_MS: u64 = 60_000;
/// How long a peer's last Network beacon keeps it present (ADR-0008 D9):
/// two and a half heartbeats, so one or two lost datagrams do not turn the
/// row grey, but a peer that went away does.
pub(crate) const NETWORK_CHANNEL_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(HEARTBEAT_INTERVAL_MS * 5 / 2);

pub(crate) const DISCOVERY_PROTOCOL: &str = "fini-device-sync-v1";
pub(crate) const DISCOVERY_PORT: u16 = 45_454;
pub(super) const DISCOVERY_TTL_SECS: u64 = 15;
pub(super) const PAIR_REQUEST_TTL_SECS: i64 = 60;
pub(crate) const SPACE_SYNC_WS_PORT: u16 = 45_455;

#[derive(Clone)]
pub struct DeviceConnectionState {
    pub identity: DeviceIdentity,
    pub db_path: PathBuf,
    pub discovery_port: u16,
    /// The UDP port the Network channel's iroh endpoint binds (the name is
    /// older than iroh; `FINI_SPACE_SYNC_WS_PORT` sets it).
    pub space_sync_ws_port: u16,
    /// This device's iroh key (ADR-0009 D8).
    pub(crate) secret_key: iroh::SecretKey,
    /// The Network channel: its iroh endpoint, mDNS lookup and exchange
    /// attempts.
    pub(crate) network: Arc<crate::services::communication::channel::network::NetworkChannel>,
    /// The Bluetooth channel: its scan, advertisement, delivery schedule and
    /// iroh endpoint. On a platform without Bluetooth it only says so.
    pub(crate) bluetooth: Arc<crate::services::communication::channel::bluetooth::BluetoothChannel>,
    /// Carries pairing frames over Bluetooth, and holds the links requests
    /// arrived on.
    pub(crate) bluetooth_carrier: Arc<carrier::BluetoothCarrier>,
    runtime: Arc<Mutex<DiscoveryRuntime>>,
    lifecycle_tx: LifecycleBus,
}

/// Hands a frame to the running exchange with this peer, if there is one.
fn forward_to_running_exchange(
    runtime: &DiscoveryRuntime,
    peer_device_id: &str,
    msg: PeerFrame,
) -> bool {
    [ChannelKind::Network, ChannelKind::Bluetooth]
        .into_iter()
        .filter_map(|kind| runtime.peer_sessions.get(&(peer_device_id.to_string(), kind)))
        .next()
        .is_some_and(|sender| sender.try_send(SessionCommand::Forward(msg)).is_ok())
}

/// The iroh key pinned for this pair (ADR-0009 D8), if any.
pub(crate) fn pinned_key(conn: &mut diesel::SqliteConnection, device_id: &str) -> Option<String> {
    use diesel::prelude::*;
    crate::schema::paired_devices::table
        .find(device_id)
        .select(crate::schema::paired_devices::endpoint_id)
        .first::<Option<String>>(conn)
        .ok()
        .flatten()
}

fn env_port(name: &str, fallback: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(fallback)
}

impl DeviceConnectionState {
    /// The carrier for a pairing exchange over Bluetooth or over Network.
    pub(crate) fn pairing_carrier(&self, via_bluetooth: bool) -> &dyn carrier::PairingCarrier {
        if via_bluetooth {
            &*self.bluetooth_carrier
        } else {
            &carrier::NetworkCarrier
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn from_app_data_dir(app_data_dir: &Path) -> Self {
        Self::from_db_path(app_data_dir, app_data_dir.join("fini.db"))
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn from_db_path(app_data_dir: &Path, db_path: PathBuf) -> Self {
        Self::try_from_db_path(app_data_dir, db_path)
            .expect("failed to create device connection state")
    }

    /// Builds the state, and starts nothing.
    ///
    /// Constructing this used to spawn the mDNS/UDP presence worker as a side
    /// effect, which meant one channel's way of finding peers began the
    /// instant shared state existed -- while the other channel's scanning
    /// began somewhere else entirely, inside its own adapter. Finding a peer
    /// belongs to the channel that knows how, so each service starts its own
    /// (`ChannelService::start_discovery`).
    pub fn try_from_db_path(app_data_dir: &Path, db_path: PathBuf) -> Result<Self, String> {
        let identity = DeviceIdentity::load_or_create(app_data_dir, &db_path)?;
        let secret_key = device_key::try_load_or_create_secret_key(&db_path)?;
        let mut identity = identity;
        identity.endpoint_id = secret_key.public().to_string();
        let bluetooth = Arc::new(crate::services::communication::channel::bluetooth::BluetoothChannel::new(&identity));

        let space_sync_ws_port = env_port("FINI_SPACE_SYNC_WS_PORT", SPACE_SYNC_WS_PORT);
        let network = Arc::new(crate::services::communication::channel::network::NetworkChannel::new(
            secret_key.clone(),
            space_sync_ws_port,
        ));

        Ok(Self {
            identity,
            db_path,
            discovery_port: env_port("FINI_DISCOVERY_PORT", DISCOVERY_PORT),
            space_sync_ws_port,
            secret_key,
            network,
            bluetooth,
            bluetooth_carrier: Arc::default(),
            runtime: Arc::new(Mutex::new(DiscoveryRuntime::default())),
            lifecycle_tx: new_lifecycle_bus(),
        })
    }

    /// The key to pin for `peer_device_id` once pairing `request_id` is
    /// saved: the one that came with that pairing, if it was for this peer.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn pairing_key(&self, request_id: &str, peer_device_id: &str) -> Option<String> {
        let guard = self.runtime.lock().ok()?;
        let (device_id, key) = guard.pairing_keys.get(request_id)?;
        (device_id == peer_device_id).then(|| key.clone())
    }

    /// The key a peer announces in its presence beacon, if it is in range.
    /// Unproven: only for dialling it, where TLS then proves it.
    pub(crate) fn presence_key(&self, device_id: &str) -> Option<String> {
        let guard = self.runtime.lock().ok()?;
        guard
            .presence
            .get(device_id)
            .or_else(|| guard.discovered.get(device_id))
            .and_then(|peer| peer.endpoint_id.clone())
    }

    /// Records a device that iroh's mDNS lookup heard on the network: its
    /// presence (the Network channel goes green for a paired device), and a
    /// row in the Add Device list while it is in Add Device mode.
    pub(crate) fn record_network_device(
        &self, device_id: &str, hostname: &str, addr: std::net::SocketAddr, endpoint_id: &str, add_device: bool,
    ) {
        let newly_present = {
            let Ok(mut guard) = self.runtime.lock() else {
                return;
            };
            let newly_present = !guard.is_present(device_id);
            let seen = types::SeenPeer {
                hostname: hostname.to_string(),
                addr: addr.ip().to_string(),
                discovery_port: 0,
                ws_port: Some(addr.port()),
                endpoint_id: Some(endpoint_id.to_string()),
                last_seen_at: runtime::utc_now(),
                last_seen_mono: std::time::Instant::now(),
            };
            guard.rx_count += 1;
            guard.presence.insert(device_id.to_string(), seen.clone());
            if add_device && guard.add_mode_enabled {
                guard.discovered.insert(device_id.to_string(), seen);
            } else {
                guard.discovered.remove(device_id);
            }
            newly_present
        };
        // A device that appears is the moment waiting work can go to it.
        if newly_present {
            crate::services::communication::sync::commands::notify_sync_work_pending();
        }
    }

    /// iroh's mDNS lookup no longer hears the device.
    pub(crate) fn remove_network_device(&self, device_id: &str) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.presence.remove(device_id);
            guard.discovered.remove(device_id);
        }
    }

    /// Keeps only recent network records; see `runtime::keep_recent_network_records`.
    pub(crate) fn keep_recent_network_records(&self) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.keep_recent_network_records();
        }
    }

    pub fn take_incoming_sync_events(&self) -> Vec<SyncEventEnvelope> {
        let Ok(mut guard) = self.runtime.lock() else {
            return Vec::new();
        };
        let mut events: Vec<SyncEventEnvelope> =
            guard.incoming_sync_events.drain().map(|(_, v)| v).collect();
        events.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        events
    }

    pub fn restore_incoming_sync_events(&self, events: Vec<SyncEventEnvelope>) {
        let Ok(mut guard) = self.runtime.lock() else {
            return;
        };

        for event in events {
            guard
                .incoming_sync_events
                .insert(event.event_id.clone(), event);
        }
    }

    pub fn take_incoming_sync_acks(&self) -> Vec<IncomingSyncAck> {
        let Ok(mut guard) = self.runtime.lock() else {
            return Vec::new();
        };
        let mut acks: Vec<IncomingSyncAck> =
            guard.incoming_sync_acks.drain().map(|(_, v)| v).collect();
        acks.sort_by(|a, b| {
            a.acked_at
                .cmp(&b.acked_at)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        acks
    }

    pub fn push_incoming_sync_event(&self, envelope: SyncEventEnvelope) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard
                .incoming_sync_events
                .insert(envelope.event_id.clone(), envelope);
        }
    }

    pub fn push_incoming_sync_ack(&self, ack: IncomingSyncAck) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.incoming_sync_acks.insert(ack.event_id.clone(), ack);
        }
    }

    pub fn push_incoming_space_mapping_update(&self, update: IncomingSpaceMappingUpdate) {
        if let Ok(mut guard) = self.runtime.lock() {
            let first_space_id = update
                .mapped_space_ids
                .first()
                .cloned()
                .unwrap_or_else(|| "none".to_string());
            let key = format!(
                "{}:{}:{}",
                update.from_device_id, first_space_id, update.sent_at
            );
            guard.incoming_space_mapping_updates.insert(key, update);
        }
    }

    pub fn push_incoming_space_sync_end(&self, update: IncomingSpaceSyncEnd) {
        if let Ok(mut guard) = self.runtime.lock() {
            let key = format!("{}:{}", update.from_device_id, update.space_id);
            guard.incoming_space_sync_ends.insert(key, update);
        }
    }

    pub fn take_incoming_space_sync_ends(&self) -> Vec<IncomingSpaceSyncEnd> {
        let Ok(mut guard) = self.runtime.lock() else {
            return Vec::new();
        };

        guard
            .incoming_space_sync_ends
            .drain()
            .map(|(_, v)| v)
            .collect()
    }

    /// Register an exchange that just authenticated (ADR-0008 D10). At most
    /// one per (peer, channel); `false` means one is already running there
    /// and the caller's link must not proceed. Frames queued for the peer
    /// while nothing was connected go out first.
    pub fn try_claim_session(&self, peer_device_id: &str, kind: ChannelKind, sender: SessionSender) -> bool {
        let queued = {
            let Ok(mut guard) = self.runtime.lock() else {
                return false;
            };
            let key = (peer_device_id.to_string(), kind);
            if guard.peer_sessions.contains_key(&key) {
                return false;
            }
            guard.peer_sessions.insert(key, sender.clone());
            guard.pending_frames.remove(peer_device_id).unwrap_or_default()
        };
        // What the new mailbox cannot take stays queued, ahead of anything
        // queued since, for this exchange's end to ask for the next one.
        let mut left_over = Vec::new();
        for frame in queued {
            if !left_over.is_empty() {
                left_over.push(frame);
                continue;
            }
            if sender.try_send(SessionCommand::Forward(frame.clone())).is_err() {
                left_over.push(frame);
            }
        }
        if !left_over.is_empty() {
            if let Ok(mut guard) = self.runtime.lock() {
                guard
                    .pending_frames
                    .entry(peer_device_id.to_string())
                    .or_default()
                    .splice(0..0, left_over);
            }
        }
        let _ = self.lifecycle_tx.send(LifecycleEvent::SessionEstablished {
            peer_device_id: peer_device_id.to_string(),
            kind,
        });
        // A connection coming up makes waiting work sendable.
        crate::services::communication::sync::commands::notify_sync_work_pending();
        true
    }

    pub fn release_session(&self, peer_device_id: &str, kind: ChannelKind) {
        let removed = {
            let Ok(mut guard) = self.runtime.lock() else {
                return;
            };
            guard.peer_sessions.remove(&(peer_device_id.to_string(), kind)).is_some()
        };
        // Frames queued while this exchange ran (its mailbox was full) wait
        // for the next one; ask for it rather than for unrelated work.
        if self.has_queued_frames(peer_device_id) {
            crate::services::communication::sync::commands::notify_sync_work_pending();
        }
        if removed {
            let _ = self.lifecycle_tx.send(LifecycleEvent::SessionEnded {
                peer_device_id: peer_device_id.to_string(),
                kind,
            });
        }
    }

    /// The channel of an exchange running with this peer right now, if any
    /// -- Network first when both are.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn active_exchange_channel(&self, peer_device_id: &str) -> Option<ChannelKind> {
        let guard = self.runtime.lock().ok()?;
        [ChannelKind::Network, ChannelKind::Bluetooth]
            .into_iter()
            .find(|kind| guard.peer_sessions.contains_key(&(peer_device_id.to_string(), *kind)))
    }

    /// Whether an exchange is running with this peer on this channel.
    pub fn has_session_on(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard.peer_sessions.contains_key(&(peer_device_id.to_string(), kind))
    }

    /// Whether this device is currently discoverable for pairing —
    /// `specs/device-connect/README.md`: "Only devices in add-mode are
    /// pairing candidates." Used by `session::run_peer_gate`'s
    /// `DiscoveryHello` handling (ADR 0002 Phase 3) to decide whether to
    /// reply at all, the BLE-scan equivalent of the existing check
    /// `receive_ws_pair_request` already makes for network `PairRequest`s.
    pub fn is_add_mode_enabled(&self) -> bool {
        self.runtime.lock().map(|guard| guard.add_mode_enabled).unwrap_or(false)
    }

    /// Test-only, instance-scoped toggle for `is_add_mode_enabled` --
    /// deliberately does *not* also flip `channel::bluetooth::set_add_mode`
    /// (unlike the real `device_connection_enter_add_mode_impl`/
    /// `leave_add_mode_impl`), since that is a *process-global* singleton
    /// shared by every test in the binary. Exercising `run_peer_gate`'s
    /// `DiscoveryHello` gating needs only this instance's flag, not the
    /// BLE-advertising side effect.
    #[cfg(test)]
    pub fn set_add_mode_for_test(&self, enabled: bool) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.add_mode_enabled = enabled;
        }
    }

    /// Test-only: injects a presence entry directly, bypassing the real
    /// mDNS discovery worker entirely. Lets a test exercise
    /// `NetworkChannel::start_exchange` (which gates on presence) without
    /// standing up real UDP broadcast traffic. `endpoint_id` is the key the
    /// peer would announce.
    #[cfg(test)]
    pub fn note_presence_for_test(
        &self, peer_device_id: &str, addr: &str, ws_port: u16, endpoint_id: Option<String>,
    ) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.presence.insert(
                peer_device_id.to_string(),
                types::SeenPeer {
                    hostname: peer_device_id.to_string(),
                    addr: addr.to_string(),
                    discovery_port: 0,
                    ws_port: Some(ws_port),
                    endpoint_id,
                    last_seen_at: crate::services::db::utc_now(),
                    last_seen_mono: std::time::Instant::now(),
                },
            );
        }
    }

    /// Test-only: makes this peer's last Network beacon `by` older.
    #[cfg(test)]
    pub fn age_presence_for_test(&self, peer_device_id: &str, by: std::time::Duration) {
        if let Ok(mut guard) = self.runtime.lock() {
            if let Some(peer) = guard.presence.get_mut(peer_device_id) {
                peer.last_seen_mono -= by;
            }
        }
    }

    /// Live channel-changed/connect/disconnect rows: `lib.rs`'s
    /// `forward_session_lifecycle_events` subscribes once at app setup and
    /// forwards each event to the frontend (ADR-0003 Phase 2).
    /// `device_connection_channel_statuses` stays the source of truth for
    /// a one-shot/polled read; this is the push side of the same signal.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn subscribe_lifecycle(&self) -> tokio::sync::broadcast::Receiver<LifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    /// Sends a frame over a running exchange with this peer (Network first).
    /// `false` when none is running; see `queue_for_peer` for frames that
    /// must still arrive.
    pub fn push_to_peer(&self, peer_device_id: &str, msg: PeerFrame) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        forward_to_running_exchange(&guard, peer_device_id, msg)
    }

    /// Sends a frame now if an exchange is running, otherwise keeps it for
    /// the next one and asks for one (ADR-0008 D10: the peer's appearing is
    /// what delivers it). In memory only: what must survive a restart lives
    /// in the outbox.
    pub fn queue_for_peer(&self, peer_device_id: &str, msg: PeerFrame) {
        // Send-or-queue under one lock, so an exchange cannot be claimed
        // between the two and miss the frame. Frames already waiting go
        // first, so this one waits behind them.
        let sent = {
            let Ok(mut guard) = self.runtime.lock() else {
                return;
            };
            let waiting = guard.pending_frames.get(peer_device_id).is_some_and(|q| !q.is_empty());
            let sent = !waiting && forward_to_running_exchange(&guard, peer_device_id, msg.clone());
            if !sent {
                guard
                    .pending_frames
                    .entry(peer_device_id.to_string())
                    .or_default()
                    .push(msg);
            }
            sent
        };
        if !sent {
            crate::services::communication::sync::commands::notify_sync_work_pending();
        }
    }

    /// Whether frames are waiting for the next exchange with this peer.
    pub fn has_queued_frames(&self, peer_device_id: &str) -> bool {
        self.runtime
            .lock()
            .map(|guard| guard.pending_frames.get(peer_device_id).is_some_and(|q| !q.is_empty()))
            .unwrap_or(false)
    }

    /// Whether an exchange is running with this peer on any channel.
    pub fn has_session(&self, peer_device_id: &str) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard
            .peer_sessions
            .keys()
            .any(|(id, _)| id == peer_device_id)
    }

    /// Ends a running exchange on this channel -- the switch was turned off,
    /// or the channel unlinked. `false` if none was running. A full mailbox
    /// does not lose the close: it waits for the next free slot, which a
    /// waiting send gets before any later frame.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn close_session_on(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        let sender = {
            let guard = match self.runtime.lock() {
                Ok(g) => g,
                Err(_) => return false,
            };
            match guard.peer_sessions.get(&(peer_device_id.to_string(), kind)) {
                Some(sender) => sender.clone(),
                None => return false,
            }
        };
        match sender.try_send(SessionCommand::Close) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(close)) => {
                tauri::async_runtime::spawn(async move {
                    let _ = sender.send(close).await;
                });
                true
            }
            // The exchange has already ended.
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// For the test switch (debug builds): ends every exchange on this
    /// channel. For Network it also forgets the devices heard on the
    /// network, as a lost network does.
    #[cfg(all(feature = "ui-plane", feature = "devtools"))]
    pub fn end_channel_for_test(&self, kind: ChannelKind) {
        let peers: Vec<String> = {
            let Ok(mut guard) = self.runtime.lock() else {
                return;
            };
            if kind == ChannelKind::Network {
                guard.presence.clear();
                guard.discovered.clear();
            }
            guard
                .peer_sessions
                .keys()
                .filter(|(_, session_kind)| *session_kind == kind)
                .map(|(peer, _)| peer.clone())
                .collect()
        };
        for peer in peers {
            self.close_session_on(&peer, kind);
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn receive_ws_pair_request(
        &self,
        payload: PairRequestPayload,
        from_addr: String,
        from_key: Option<String>,
        via_bluetooth: bool,
    ) -> Result<(), String> {
        if payload.to_device_id != self.identity.device_id {
            return Ok(());
        }

        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;
        guard.rx_count += 1;

        if guard.add_mode_enabled {
            let is_new = !guard
                .incoming_requests
                .contains_key(payload.request_id.as_str());
            guard.incoming_requests.insert(
                payload.request_id.clone(),
                types::StoredIncomingPairRequest::from_payload(&payload, from_addr, from_key, via_bluetooth),
            );

            if is_new {
                eprintln!(
                    "[device-sync] incoming ws pair request {} from {} ({})",
                    payload.request_id, payload.from_hostname, payload.from_device_id
                );
            }
        }

        Ok(())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn receive_ws_pair_accept(
        &self,
        payload: PairAcceptPayload,
    ) -> Result<(), String> {
        if payload.to_device_id != self.identity.device_id {
            return Ok(());
        }

        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;
        guard.rx_count += 1;
        guard.outgoing_code_updates.insert(
            payload.request_id.clone(),
            PairCodeUpdate {
                request_id: payload.request_id,
                code: payload.code,
                accepted_at: payload.accepted_at,
            },
        );
        Ok(())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn receive_ws_pair_complete(
        &self,
        payload: PairCompletePayload,
        from_addr: String,
        from_key: Option<String>,
        via_bluetooth: bool,
    ) -> Result<(), String> {
        if payload.to_device_id != self.identity.device_id {
            return Ok(());
        }
        // When `via_bluetooth`, trust the address actually observed on this
        // connection over the sender's self-reported `payload.bluetooth_address`
        // -- same reasoning as `IncomingPairRequest::from_bluetooth_address`.
        let bluetooth_address = if via_bluetooth {
            Some(from_addr)
        } else {
            payload.bluetooth_address.clone()
        };

        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| "device sync runtime lock poisoned".to_string())?;
        guard.rx_count += 1;
        if let Some(key) = from_key {
            guard
                .pairing_keys
                .insert(payload.request_id.clone(), (payload.from_device_id.clone(), key));
        }
        guard.outgoing_pair_completions.insert(
            payload.request_id.clone(),
            PairCompletionUpdate {
                request_id: payload.request_id,
                from_device_id: payload.from_device_id,
                from_hostname: payload.from_hostname,
                paired_at: payload.paired_at,
                via_bluetooth,
                bluetooth_address,
            },
        );
        Ok(())
    }

    /// Raw discovery presence: is this peer's beacon reaching us right now?
    /// ADR-0003 revision: this is now the *only* network-availability
    /// signal that matters for dialing -- `NetworkChannel::start_exchange` dials
    /// unconditionally whenever a peer is presenced and has no session on
    /// this channel yet, rather than withdrawing in favor of Bluetooth.
    /// Consulted by `ChannelStatusCode::NetworkUnavailable`'s gray-row
    /// determination too.
    pub fn network_peer_available(&self, peer_device_id: &str) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard
            .presence
            .get(peer_device_id)
            .is_some_and(|peer| peer.last_seen_mono.elapsed() < NETWORK_CHANNEL_TIMEOUT)
    }

    /// Whether the Network channel cannot look for devices; see
    /// `DiscoveryRuntime::network_discovery_stopped`.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn network_discovery_stopped(&self) -> bool {
        self.runtime
            .lock()
            .map(|guard| guard.network_discovery_stopped)
            .unwrap_or(false)
    }

    pub(crate) fn set_network_discovery_stopped(&self, failing: bool) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.network_discovery_stopped = failing;
        }
    }

    /// Start a setup search for this peer's channel (ADR-0008 D2). While it
    /// runs, this device answers the peer's hello on that channel; nothing
    /// else makes it answer. Starting again keeps the progress so far.
    #[cfg(any(feature = "ui-plane", test))]
    /// Returns the attempt it joined or started (see `ChannelSetup::attempt`).
    pub fn begin_channel_setup(&self, peer_device_id: &str, kind: ChannelKind) -> u64 {
        static NEXT_ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let Ok(mut guard) = self.runtime.lock() else {
            return 0;
        };
        guard
            .channel_setups
            .entry((peer_device_id.to_string(), kind))
            .or_insert_with(|| ChannelSetup {
                attempt: NEXT_ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                ..ChannelSetup::default()
            })
            .attempt
    }

    /// Whether any setup search is running on this channel.
    pub fn any_channel_setup(&self, kind: ChannelKind) -> bool {
        self.runtime
            .lock()
            .map(|guard| guard.channel_setups.keys().any(|(_, k)| *k == kind))
            .unwrap_or(false)
    }

    /// End the setup search, returning how far its init got.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn end_channel_setup(&self, peer_device_id: &str, kind: ChannelKind) -> Option<ChannelSetup> {
        let mut guard = self.runtime.lock().ok()?;
        guard.channel_setups.remove(&(peer_device_id.to_string(), kind))
    }

    /// The setup search running for this peer's channel, if any.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn channel_setup(&self, peer_device_id: &str, kind: ChannelKind) -> Option<ChannelSetup> {
        let guard = self.runtime.lock().ok()?;
        guard.channel_setups.get(&(peer_device_id.to_string(), kind)).copied()
    }

    /// Record one half of an init. A no-op when no setup search is running
    /// for that channel: an init only counts while both people are at it.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn note_channel_setup(
        &self,
        peer_device_id: &str,
        kind: ChannelKind,
        update: impl FnOnce(&mut ChannelSetup),
    ) {
        if let Ok(mut guard) = self.runtime.lock() {
            if let Some(setup) = guard.channel_setups.get_mut(&(peer_device_id.to_string(), kind)) {
                update(setup);
            }
        }
    }

    /// `note_channel_setup`, but only while `attempt` is still the running
    /// one: a result from a search that was closed does not count for the
    /// next.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn note_channel_setup_attempt(
        &self,
        peer_device_id: &str,
        kind: ChannelKind,
        attempt: u64,
        update: impl FnOnce(&mut ChannelSetup),
    ) {
        self.note_channel_setup(peer_device_id, kind, |setup| {
            if setup.attempt == attempt {
                update(setup);
            }
        });
    }

    /// Where the Network channel last heard this peer's presence beacon.
    pub fn network_presence_address(&self, peer_device_id: &str) -> Option<(String, u16)> {
        let guard = self.runtime.lock().ok()?;
        guard
            .presence
            .get(peer_device_id)
            .filter(|peer| peer.last_seen_mono.elapsed() < NETWORK_CHANNEL_TIMEOUT)
            .map(|peer| (peer.addr.clone(), peer.ws_port.unwrap_or(self.space_sync_ws_port)))
    }
}
