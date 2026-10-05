mod commands;
mod device_key;
mod runtime;
pub(crate) mod channel_status;
pub(crate) mod channels;
#[cfg(any(feature = "ui-plane", test))]
pub(crate) mod setup;
#[cfg(any(feature = "ui-plane", test))]
pub(crate) mod gate;
pub(crate) mod types;

use std::net::Ipv4Addr;
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
    device_connection_discover_bluetooth_candidates, device_connection_discovery_snapshot,
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

#[cfg(any(feature = "ui-plane", test))]
pub use gate::run_peer_gate;

pub use commands::bluetooth_dial_candidates;
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
use runtime::{spawn_discovery_worker, try_load_or_create_identity};
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

pub const DISCOVERY_INTERVAL_MS: u64 = 5_000;
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
pub(super) const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
pub(crate) const SPACE_SYNC_WS_PORT: u16 = 45_455;
pub(crate) const MDNS_SERVICE_TYPE: &str = "_fini-sync._udp.local.";

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
    /// The Network channel's iroh endpoint, bound on first use
    /// (`channel::network`). Per state, not per process: tests run two
    /// devices in one process.
    pub(crate) network_endpoint: Arc<tokio::sync::OnceCell<iroh::Endpoint>>,
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

fn env_port_list(name: &str, fallback: u16) -> Vec<u16> {
    let mut ports: Vec<u16> = std::env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| item.trim().parse::<u16>().ok())
                .collect()
        })
        .unwrap_or_default();

    if !ports.contains(&fallback) {
        ports.push(fallback);
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}

impl DeviceConnectionState {
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
        let identity = try_load_or_create_identity(app_data_dir, &db_path)?;
        let secret_key = device_key::try_load_or_create_secret_key(&db_path)?;
        let mut identity = identity;
        identity.endpoint_id = secret_key.public().to_string();

        Ok(Self {
            identity,
            db_path,
            discovery_port: env_port("FINI_DISCOVERY_PORT", DISCOVERY_PORT),
            space_sync_ws_port: env_port("FINI_SPACE_SYNC_WS_PORT", SPACE_SYNC_WS_PORT),
            secret_key,
            network_endpoint: Arc::new(tokio::sync::OnceCell::new()),
            runtime: Arc::new(Mutex::new(DiscoveryRuntime::default())),
            lifecycle_tx: new_lifecycle_bus(),
        })
    }

    /// Remembers the key a peer proved over TLS on a connection to this
    /// device, so a pairing that completes can pin it. Links without a key
    /// (none today on the Network channel) are ignored.
    pub(crate) fn note_link_key(&self, device_id: &str, key: Option<String>) {
        let Some(key) = key else {
            return;
        };
        if let Ok(mut guard) = self.runtime.lock() {
            guard.link_keys.insert(device_id.to_string(), key);
        }
    }

    /// The key `device_id` proved on its latest connection to this device.
    pub(crate) fn link_key(&self, device_id: &str) -> Option<String> {
        self.runtime.lock().ok()?.link_keys.get(device_id).cloned()
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

    /// Start the mDNS/UDP presence worker, once per state.
    ///
    /// `pub(crate)` and called only from `NetworkChannelService`: the worker
    /// needs this state's private identity and presence store, but *whether
    /// and when* the Network channel starts looking is the service's call.
    ///
    /// Guarded on this state's own `worker_started` rather than a process
    /// static, so asking twice is harmless while two states in one process
    /// still each get their own worker. That matters for tests, where a
    /// process-wide guard would let the first state ever created decide for
    /// every one after it.
    pub(crate) fn start_network_discovery(&self) {
        {
            let Ok(guard) = self.runtime.lock() else {
                return;
            };
            if guard.worker_started {
                return;
            }
        }
        spawn_discovery_worker(
            self.identity.clone(),
            self.runtime.clone(),
            self.discovery_port,
            env_port_list("FINI_DISCOVERY_PEER_PORTS", self.discovery_port),
            self.space_sync_ws_port,
        );
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
    #[cfg(any(feature = "ui-plane", test))]
    pub fn is_add_mode_enabled(&self) -> bool {
        self.runtime.lock().map(|guard| guard.add_mode_enabled).unwrap_or(false)
    }

    /// Test-only, instance-scoped toggle for `is_add_mode_enabled` --
    /// deliberately does *not* also flip `channel::ble::set_add_mode`
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
    /// `channel::network::start_exchange` (which gates on presence) without
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
        self.note_link_key(&payload.from_device_id, from_key.clone());

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
                runtime::build_incoming_pair_request(&payload, from_addr, from_key, via_bluetooth),
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
        from_key: Option<String>,
    ) -> Result<(), String> {
        if payload.to_device_id != self.identity.device_id {
            return Ok(());
        }
        self.note_link_key(&payload.from_device_id, from_key);

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
        self.note_link_key(&payload.from_device_id, from_key);

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
    /// signal that matters for dialing -- `network::start_exchange` dials
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

    /// Whether the last Network presence beacon failed to go out at all.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn network_broadcast_failing(&self) -> bool {
        self.runtime
            .lock()
            .map(|guard| guard.network_broadcast_failing)
            .unwrap_or(false)
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
