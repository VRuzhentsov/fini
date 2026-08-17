mod commands;
mod runtime;
mod transport;
pub(crate) mod types;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::services::space_sync::types::{PeerFrame, SessionCommand, SessionSender, SyncEventEnvelope};
use crate::services::transport::selection::{new_lifecycle_bus, LifecycleBus, LifecycleEvent};
use crate::services::transport::TransportKind;

// Shared with `transport::tests`, which sets/clears the same process-global
// `FINI_BLUETOOTH_PAIRED_ADDRESSES` env var in its own tests -- see the
// lock's own doc comment for why this must be one lock, not two.
#[cfg(test)]
pub(crate) use commands::BLUETOOTH_PAIRED_ADDRESSES_ENV_LOCK;

#[cfg(any(feature = "ui-plane", test))]
pub use commands::{
    device_connection_consume_space_mapping_updates, device_connection_debug_status,
    device_connection_discover_bluetooth_candidates, device_connection_discovery_snapshot,
    device_connection_enter_add_mode, device_connection_find_bluetooth_address,
    device_connection_get_identity, device_connection_get_paired_devices,
    device_connection_leave_add_mode, device_connection_pair_accept_request,
    device_connection_pair_acknowledge_request, device_connection_pair_complete_request,
    device_connection_pair_incoming_requests, device_connection_pair_outgoing_completions,
    device_connection_pair_outgoing_updates, device_connection_presence_snapshot,
    device_connection_save_paired_device, device_connection_send_pair_request,
    device_connection_send_pair_request_bluetooth, device_connection_session_transport,
    device_connection_set_bluetooth_transport, device_connection_set_preferred_transport,
    device_connection_transport_liveness, device_connection_transport_statuses, device_connection_unpair,
    device_connection_update_last_seen,
};
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use commands::bluetooth_dial_candidates;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) use commands::bluetooth_address_is_os_paired;
pub(crate) use commands::{
    local_bluetooth_address, normalize_bluetooth_address, peer_transport_preference,
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
    device_connection_send_pair_request_impl, device_connection_session_transport_impl,
    device_connection_set_bluetooth_transport_impl, device_connection_set_bluetooth_transport_with_state_impl,
    device_connection_set_preferred_transport_impl,
    device_connection_transport_liveness_impl, device_connection_transport_statuses_impl,
    device_connection_unpair_impl, device_connection_update_last_seen_impl,
};
use runtime::{spawn_discovery_worker, try_load_or_create_identity};
// `RowTransportKind`, not bare `TransportKind`: this crate already has
// `crate::services::transport::TransportKind` (TcpWs/Sim/Bluetooth/LoRa),
// and re-exporting this module's own coarser Network/Bluetooth enum under
// the same bare name would collide for any external caller that needs
// both in scope at once (e.g. `transport::tests`, which imports the
// fine-grained one already).
pub use transport::{
    build_transport_statuses, TransportKind as RowTransportKind, TransportLiveness, TransportStatus,
    TransportStatusCode, TransportStatusInputs,
};
use types::DiscoveryRuntime;
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

pub(crate) const DISCOVERY_PROTOCOL: &str = "fini-device-sync-v1";
pub(crate) const DISCOVERY_PORT: u16 = 45_454;
pub(super) const DISCOVERY_TTL_SECS: u64 = 15;
pub(super) const PAIR_REQUEST_TTL_SECS: i64 = 60;
pub(super) const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
pub(crate) const SPACE_SYNC_WS_PORT: u16 = 45_455;
pub(crate) const MDNS_SERVICE_TYPE: &str = "_fini-sync._tcp.local.";

#[derive(Clone)]
pub struct DeviceConnectionState {
    pub identity: DeviceIdentity,
    pub db_path: PathBuf,
    pub discovery_port: u16,
    pub space_sync_ws_port: u16,
    runtime: Arc<Mutex<DiscoveryRuntime>>,
    lifecycle_tx: LifecycleBus,
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

    pub fn from_db_path(app_data_dir: &Path, db_path: PathBuf) -> Self {
        Self::try_from_db_path(app_data_dir, db_path)
            .expect("failed to create device connection state")
    }

    pub fn try_from_db_path(app_data_dir: &Path, db_path: PathBuf) -> Result<Self, String> {
        let identity = try_load_or_create_identity(app_data_dir, &db_path)?;
        let runtime = Arc::new(Mutex::new(DiscoveryRuntime::default()));
        let discovery_port = env_port("FINI_DISCOVERY_PORT", DISCOVERY_PORT);
        let space_sync_ws_port = env_port("FINI_SPACE_SYNC_WS_PORT", SPACE_SYNC_WS_PORT);
        let discovery_broadcast_ports = env_port_list("FINI_DISCOVERY_PEER_PORTS", discovery_port);

        spawn_discovery_worker(
            identity.clone(),
            runtime.clone(),
            discovery_port,
            discovery_broadcast_ports,
            space_sync_ws_port,
        );

        Ok(Self {
            identity,
            db_path,
            discovery_port,
            space_sync_ws_port,
            runtime,
            lifecycle_tx: new_lifecycle_bus(),
        })
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

    /// ADR-0003 revision: sessions are claimed *per transport*, not one per
    /// peer -- both Network and Bluetooth can be (and normally are) live at
    /// once. Succeeds (and claims this specific `(peer, kind)` slot) only if
    /// no session already exists on *this* transport for `peer_device_id`;
    /// a session already live on the *other* transport is no longer a
    /// reason to refuse. Callers must check the return value the same way
    /// as before -- on `false`, the caller's link must not proceed to
    /// `AuthOk`/the session loop.
    pub fn try_claim_session(
        &self,
        peer_device_id: &str,
        kind: TransportKind,
        sender: SessionSender,
        db_path: &Path,
    ) -> bool {
        {
            let Ok(mut guard) = self.runtime.lock() else {
                return false;
            };
            let key = (peer_device_id.to_string(), kind);
            if guard.peer_sessions.contains_key(&key) {
                return false;
            }
            guard.peer_sessions.insert(key.clone(), sender);
            guard.peer_transport_ack.insert(key, types::TransportAckState::default());
            let (pinned_to_bluetooth, bluetooth_enabled) =
                Self::bluetooth_primary_eligibility(db_path, peer_device_id);
            self.recompute_primary_locked(&mut guard, peer_device_id, pinned_to_bluetooth, bluetooth_enabled);
        }
        let _ = self.lifecycle_tx.send(LifecycleEvent::SessionEstablished {
            peer_device_id: peer_device_id.to_string(),
            kind,
        });
        true
    }

    pub fn release_session(&self, peer_device_id: &str, kind: TransportKind, db_path: &Path) {
        {
            let Ok(mut guard) = self.runtime.lock() else {
                return;
            };
            let key = (peer_device_id.to_string(), kind);
            if guard.peer_sessions.remove(&key).is_none() {
                return; // wasn't claimed on this transport; nothing to release
            }
            guard.peer_transport_ack.remove(&key);
            let (pinned_to_bluetooth, bluetooth_enabled) =
                Self::bluetooth_primary_eligibility(db_path, peer_device_id);
            self.recompute_primary_locked(&mut guard, peer_device_id, pinned_to_bluetooth, bluetooth_enabled);
        }
        let _ = self.lifecycle_tx.send(LifecycleEvent::SessionEnded {
            peer_device_id: peer_device_id.to_string(),
            kind,
        });
    }

    /// Reads `preferred_transport`/`bluetooth_enabled` for `peer_device_id`
    /// from `db_path` -- a plain, explicit path, not `self.db_path`: every
    /// other DB-touching helper in this module (`check_paired`, etc.) takes
    /// the caller's own `db_path` rather than trusting a field on `self`,
    /// and callers here (dial loops, `run_peer_gate`/`run_session`) already
    /// have the correct one in scope from their own parameters.
    fn bluetooth_primary_eligibility(db_path: &Path, peer_device_id: &str) -> (bool, bool) {
        tokio::task::block_in_place(|| {
            let mut conn = crate::services::db::open_db_at_path(db_path);
            let pinned_to_bluetooth =
                commands::peer_transport_preference(&mut conn, peer_device_id).as_deref() == Some("bluetooth");
            let bluetooth_enabled = commands::peer_bluetooth_enabled(&mut conn, peer_device_id);
            (pinned_to_bluetooth, bluetooth_enabled)
        })
    }

    /// Recomputes and stores which transport is *primary* for
    /// `peer_device_id`, given which transports currently have a claimed
    /// session, `pinned_to_bluetooth` (the caller's own read of
    /// `preferred_transport`), and `bluetooth_enabled` (the caller's own
    /// read of the pair's Bluetooth toggle). Network wins whenever it's
    /// connected, unless the pair is explicitly pinned to Bluetooth and
    /// Bluetooth is connected. A pin to a transport that isn't connected
    /// yet falls back to whatever *is* connected -- there's nothing to make
    /// primary otherwise.
    ///
    /// `bluetooth_enabled` excludes Bluetooth from candidacy entirely
    /// (neither pinned nor fallback) regardless of whether a session for it
    /// still happens to be in `peer_sessions` -- a P1 review finding on
    /// this PR: disabling Bluetooth while a session is already live doesn't
    /// synchronously remove it (`close_session_on` tears it down
    /// asynchronously, via the mailbox), so without this check a session
    /// disabled moments ago could still win the fallback race and have
    /// `push_to_peer` resume real traffic over a transport the user just
    /// turned off. This check is what actually closes that race -- the
    /// async teardown then just catches up and removes the now-provably-
    /// never-primary entry from `peer_sessions` shortly after.
    ///
    /// Called after every claim or release (and, via `refresh_primary`,
    /// every manual pin change), so this is always a pure function of
    /// current connection state, not something that can drift or need
    /// manual invalidation.
    fn recompute_primary_locked(
        &self,
        guard: &mut types::DiscoveryRuntime,
        peer_device_id: &str,
        pinned_to_bluetooth: bool,
        bluetooth_enabled: bool,
    ) {
        let network_connected = guard
            .peer_sessions
            .contains_key(&(peer_device_id.to_string(), TransportKind::TcpWs));
        // `bluetooth_enabled` only ever gates the real `Bluetooth` kind --
        // `Sim`/`LoRa` aren't governed by that DB column at all (the
        // accept/dial gates never check it for them either, see
        // `run_peer_gate`'s `kind == TransportKind::Bluetooth` guard), so
        // excluding them here would silently break every Sim-based test's
        // existing assumption that a claimed Sim session can become
        // primary.
        let bluetooth_connected = [TransportKind::Bluetooth, TransportKind::Sim, TransportKind::LoRa]
            .into_iter()
            .filter(|kind| *kind != TransportKind::Bluetooth || bluetooth_enabled)
            .find(|kind| guard.peer_sessions.contains_key(&(peer_device_id.to_string(), *kind)));

        let pick = if pinned_to_bluetooth && bluetooth_connected.is_some() {
            bluetooth_connected
        } else if network_connected {
            Some(TransportKind::TcpWs)
        } else {
            bluetooth_connected
        };

        match pick {
            Some(kind) => {
                guard.peer_primary_transport.insert(peer_device_id.to_string(), kind);
            }
            None => {
                guard.peer_primary_transport.remove(peer_device_id);
            }
        }
    }

    /// Re-runs primary-transport selection for `peer_device_id` right now,
    /// without waiting for the next claim/release event. The only external
    /// caller is `device_connection_set_preferred_transport_impl`: a manual
    /// pin change must be reflected immediately (both rows already
    /// connected, nothing to reconnect), not only whenever a transport
    /// happens to reconnect next.
    pub fn refresh_primary(&self, peer_device_id: &str, pinned_to_bluetooth: bool, bluetooth_enabled: bool) {
        let Ok(mut guard) = self.runtime.lock() else { return };
        self.recompute_primary_locked(&mut guard, peer_device_id, pinned_to_bluetooth, bluetooth_enabled);
    }

    /// Which transport is *primary* for this peer right now -- the one
    /// carrying real application traffic, reported as `RowState::Live`.
    /// `None` means neither transport is currently connected. Exposed via
    /// `device_connection_session_transport`. Renamed from the old
    /// `session_kind` (ADR-0003 revision: there can be a session on each
    /// transport at once now, so "the" session no longer names a single
    /// thing -- this specifically means the *primary* one).
    pub fn primary_transport(&self, peer_device_id: &str) -> Option<TransportKind> {
        let guard = self.runtime.lock().ok()?;
        guard.peer_primary_transport.get(peer_device_id).copied()
    }

    /// Whether a session is currently claimed on this specific transport
    /// for this peer -- independent of whether it's primary. Dial loops use
    /// this (not `primary_transport`) to decide whether they still need to
    /// keep trying: each transport now dials/connects independently of the
    /// other's state.
    pub fn has_session_on(&self, peer_device_id: &str, kind: TransportKind) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard.peer_sessions.contains_key(&(peer_device_id.to_string(), kind))
    }

    /// `run_session`'s ping-interval tick, called just before it sends a
    /// fresh `Ping`: accounts for an unanswered previous ping and a stale
    /// inbound-ping streak (see `TransportAckState`'s doc comment for the
    /// exact 3-miss decay rule), then marks a ping as newly outstanding.
    /// No-op if this (peer, transport) has no claimed session -- the
    /// session may have just ended between the tick firing and this call.
    pub(super) fn note_ping_tick(&self, peer_device_id: &str, kind: TransportKind) {
        let Ok(mut guard) = self.runtime.lock() else { return };
        let Some(ack) = guard.peer_transport_ack.get_mut(&(peer_device_id.to_string(), kind)) else {
            return;
        };
        if ack.own_ping_awaiting_pong {
            ack.consecutive_missed_own_pings += 1;
            if ack.consecutive_missed_own_pings >= 3 {
                ack.own_ping_acked = false;
            }
        }
        ack.ticks_since_peer_ping += 1;
        if ack.ticks_since_peer_ping >= 3 {
            ack.peer_ping_received = false;
        }
        ack.own_ping_awaiting_pong = true;
    }

    /// A `Pong` answering this device's own outstanding `Ping` arrived.
    pub(super) fn note_pong_received(&self, peer_device_id: &str, kind: TransportKind) {
        let Ok(mut guard) = self.runtime.lock() else { return };
        if let Some(ack) = guard.peer_transport_ack.get_mut(&(peer_device_id.to_string(), kind)) {
            ack.own_ping_acked = true;
            ack.own_ping_awaiting_pong = false;
            ack.consecutive_missed_own_pings = 0;
        }
    }

    /// An inbound `Ping` from the peer arrived (the caller replies with a
    /// `Pong` separately -- this just records the proof).
    pub(super) fn note_ping_received(&self, peer_device_id: &str, kind: TransportKind) {
        let Ok(mut guard) = self.runtime.lock() else { return };
        if let Some(ack) = guard.peer_transport_ack.get_mut(&(peer_device_id.to_string(), kind)) {
            ack.peer_ping_received = true;
            ack.ticks_since_peer_ping = 0;
        }
    }

    /// Green: both directions of the ping/ack exchange are currently
    /// proven for this (peer, transport). `false` (amber) whenever a
    /// session is claimed but the bidirectional proof hasn't completed yet
    /// or has lapsed; also `false` if there's no session on this transport
    /// at all (gray -- callers distinguish gray from amber via
    /// `has_session_on`).
    pub fn transport_reliable(&self, peer_device_id: &str, kind: TransportKind) -> bool {
        let Ok(guard) = self.runtime.lock() else { return false };
        guard
            .peer_transport_ack
            .get(&(peer_device_id.to_string(), kind))
            .map(|ack| ack.own_ping_acked && ack.peer_ping_received)
            .unwrap_or(false)
    }

    /// The amber reason for this (peer, transport)'s connected-but-not-yet-
    /// green state, or `None` once it's actually green. `None` is also
    /// returned (meaningless to the caller either way) when there's no
    /// session on this transport at all -- callers only call this once
    /// `has_session_on` is already known true, matching how
    /// `build_transport_statuses` uses it.
    pub fn transport_liveness_code(
        &self,
        peer_device_id: &str,
        kind: TransportKind,
    ) -> Option<transport::TransportStatusCode> {
        let guard = self.runtime.lock().ok()?;
        let ack = guard.peer_transport_ack.get(&(peer_device_id.to_string(), kind))?;
        if ack.own_ping_acked && ack.peer_ping_received {
            return None;
        }
        let never_proven = !ack.own_ping_acked
            && !ack.peer_ping_received
            && ack.consecutive_missed_own_pings == 0
            && ack.ticks_since_peer_ping == 0;
        if never_proven {
            Some(transport::TransportStatusCode::AwaitingFirstAck)
        } else {
            Some(transport::TransportStatusCode::PingMissed {
                count: ack.consecutive_missed_own_pings.max(ack.ticks_since_peer_ping),
            })
        }
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
    /// deliberately does *not* also flip `transport::ble::set_add_mode`
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
    /// `transport::tcp_ws::dial_with_backoff`/`spawn_dial_loop` (both gate
    /// on `list_presenced_peers`) without standing up real UDP broadcast
    /// traffic.
    #[cfg(test)]
    pub fn note_presence_for_test(&self, peer_device_id: &str, addr: &str, ws_port: u16) {
        if let Ok(mut guard) = self.runtime.lock() {
            guard.presence.insert(
                peer_device_id.to_string(),
                types::SeenPeer {
                    hostname: peer_device_id.to_string(),
                    addr: addr.to_string(),
                    discovery_port: 0,
                    ws_port: Some(ws_port),
                    last_seen_at: crate::services::db::utc_now(),
                    last_seen_mono: std::time::Instant::now(),
                },
            );
        }
    }

    /// Live transport-changed/connect/disconnect rows: `lib.rs`'s
    /// `forward_session_lifecycle_events` subscribes once at app setup and
    /// forwards each event to the frontend (ADR-0003 Phase 2).
    /// `device_connection_transport_statuses` stays the source of truth for
    /// a one-shot/polled read; this is the push side of the same signal.
    pub fn subscribe_lifecycle(&self) -> tokio::sync::broadcast::Receiver<LifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    /// Sends application traffic (SyncEvent, BootstrapStart, etc.) over the
    /// peer's *primary* transport. `Ping`/`Pong` don't go through this --
    /// `run_session`'s ping/ack loop already owns its `Link` directly and
    /// sends on it inline, since every connected transport exchanges those
    /// on its own, not just the primary one.
    pub fn push_to_peer(&self, peer_device_id: &str, msg: PeerFrame) -> bool {
        let guard = match self.runtime.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let Some(kind) = guard.peer_primary_transport.get(peer_device_id).copied() else {
            return false;
        };
        match guard.peer_sessions.get(&(peer_device_id.to_string(), kind)) {
            Some(sender) => sender.try_send(SessionCommand::Forward(msg)).is_ok(),
            None => false,
        }
    }

    /// True if the peer has a claimed session on *any* transport.
    /// ADR-0003 revision: with both transports independently connectable,
    /// "has a session" no longer implies a single transport -- callers that
    /// need to know *which* transport should use `has_session_on` or
    /// `primary_transport` instead.
    pub fn has_session(&self, peer_device_id: &str) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard
            .peer_sessions
            .keys()
            .any(|(id, _)| id == peer_device_id)
    }

    /// Forces the peer's currently claimed session on this specific
    /// transport closed, without a transport-level failure. The only
    /// caller is `device_connection_set_bluetooth_transport_with_state_impl`'s
    /// disable path: a still-open Bluetooth (or Sim, its test stand-in)
    /// session must actually stop -- not just stop counting toward primary
    /// selection (`recompute_primary_locked` already excludes a disabled
    /// pair's Bluetooth from that, closing the race between this and the
    /// async teardown below) -- to honor `specs/device-connect/README.md`'s
    /// "disabling ... prevents future Bluetooth use" contract. Best-effort:
    /// `false` just means there was no live session on this transport to
    /// close, not an error. Fire-and-forget via the mailbox -- the actual
    /// teardown (and `release_session`) happens once `run_session`'s loop
    /// processes it, not synchronously with this call.
    pub fn close_session_on(&self, peer_device_id: &str, kind: TransportKind) -> bool {
        let guard = match self.runtime.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        match guard.peer_sessions.get(&(peer_device_id.to_string(), kind)) {
            Some(sender) => sender.try_send(SessionCommand::Close).is_ok(),
            None => false,
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn receive_ws_pair_request(
        &self,
        payload: PairRequestPayload,
        from_addr: String,
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
                runtime::build_incoming_pair_request(&payload, from_addr, via_bluetooth),
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
    pub fn receive_ws_pair_accept(&self, payload: PairAcceptPayload) -> Result<(), String> {
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

    /// Returns (device_id, addr, ws_port) for every presenced peer (seen within TTL).
    pub fn list_presenced_peers(&self) -> Vec<(String, String, u16)> {
        let Ok(guard) = self.runtime.lock() else {
            return Vec::new();
        };
        guard
            .presence
            .iter()
            .map(|(id, peer)| {
                (
                    id.clone(),
                    peer.addr.clone(),
                    peer.ws_port.unwrap_or(self.space_sync_ws_port),
                )
            })
            .collect()
    }

    /// Raw discovery presence: is this peer's beacon reaching us right now?
    /// ADR-0003 revision: this is now the *only* network-availability
    /// signal that matters for dialing -- `tcp_ws::spawn_dial_loop` dials
    /// unconditionally whenever a peer is presenced and has no session on
    /// this transport yet, rather than withdrawing in favor of Bluetooth.
    /// Consulted by `TransportStatusCode::NetworkUnavailable`'s gray-row
    /// determination too.
    pub fn network_peer_available(&self, peer_device_id: &str) -> bool {
        let Ok(guard) = self.runtime.lock() else {
            return false;
        };
        guard.presence.contains_key(peer_device_id)
    }
}
