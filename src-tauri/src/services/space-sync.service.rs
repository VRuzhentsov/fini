//! SpaceSync (ADR-0010): carries the changes of mapped spaces to paired
//! devices.
//!
//! It owns the sync sessions (one for each device and channel kind), when
//! sync runs (the wake signal, the keeper), and which green channel a frame
//! goes over (the primary first). It accepts the sync connections that
//! peers open (`SessionHandler`, ALPN `fini/sync/4`) and holds the ones the
//! channels open.
//!
//! Work for a device with no green channel waits in the outbox: the
//! channels connect by themselves (`ChannelService::keep_serving`), and a
//! session that comes up wakes SpaceSync.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::models::peer_frame::{CustomSpaceDescriptor, PeerFrame, SyncEventEnvelope};
use crate::models::pairing::{IncomingSpaceMappingUpdate, IncomingSpaceSyncEnd, IncomingSyncAck};
use crate::models::{Event, Identity};
use crate::repositories::{ChannelRepository, DeviceRepository, OutboxRepository, SpaceMappingRepository};
use crate::schema::{spaces, sync_acks, sync_outbox, sync_seen, tombstones};
use crate::services::bluetooth_channel_service::BluetoothChannelService;
use crate::services::channel_service::{recv_frame, send_frame, ChannelKind, ChannelService, DataLink, SessionHandler};
use crate::services::db::{open_db_at_path, utc_now};
use crate::services::event_bus_service::EventBusService;
use crate::services::network_channel_service::NetworkChannelService;

/// QUIC close codes of a session the peer rejected, which the connecting
/// side reads. There is no rejection frame (2026-10-13: TLS only).
pub const CLOSE_NOT_PAIRED: u32 = 1;
pub const CLOSE_CHANNEL_OFF: u32 = 2;
/// Both devices connected at once; the smaller device id's connection is
/// kept.
pub const CLOSE_CROSSED: u32 = 3;
pub const CLOSE_SESSION_ACTIVE: u32 = 4;
/// The pair is there, but the peer has no row for this channel: removed
/// there, or not set up yet. The connecting side removes its own row,
/// unless a setup of the channel is open there.
pub const CLOSE_CHANNEL_MISSING: u32 = 5;

/// Whether the peer closed a session with `close_code` because it did not
/// want it (not paired there, or the channel is off or missing there): the
/// connecting side counts that as a failed attempt and waits before it
/// tries again. A crossing or an active session is no failure: the peer's
/// own session carries the work.
pub fn rejected_by_peer(close_code: Option<u32>) -> bool {
    matches!(close_code, Some(CLOSE_NOT_PAIRED | CLOSE_CHANNEL_OFF | CLOSE_CHANNEL_MISSING))
}

/// How long after this device's own connection to a peer became a session
/// a connection from that peer counts as a crossing. Later, it is a peer
/// that started again, and replaces the session.
pub const CROSSING_WINDOW: Duration = Duration::from_secs(3);

/// How long a new connection waits for the held session it replaces to
/// release its slot.
const HELD_SESSION_RELEASE_WAIT: Duration = Duration::from_secs(3);

/// The longest an idle, fully connected pair goes without a tick on
/// Android. Every moment that makes work possible wakes SpaceSync, so this
/// is a floor, not the time a change takes.
#[cfg(target_os = "android")]
const TICK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How long the keeper waits after a wake before it ticks: frames arrive
/// together, and each one wakes, so a burst becomes one tick.
const KEEPER_SETTLE: Duration = Duration::from_millis(150);

/// The shortest time between two sends of the same unacked change to the
/// same device, so that a change slow to be acked is not sent on every
/// tick.
const EVENT_RESEND_COOLDOWN: Duration = Duration::from_secs(5);

/// How many changes go to one device in one tick.
const MAX_EVENTS_PER_PEER_PER_TICK: usize = 64;

/// How soon to ask again for bootstrap requests that a full session
/// mailbox did not take.
const BOOTSTRAP_RETRY: Duration = Duration::from_secs(2);

/// How soon after a held session ends the channel tries to connect again.
const HELD_SESSION_RECONNECT: Duration = Duration::from_secs(2);

/// How long after a session ends the keeper looks again for work it left
/// unfinished.
const EXCHANGE_END_RECHECK: Duration = Duration::from_secs(60);

/// Why a sync request was rejected: shown to the person as it is.
pub const PEER_NOT_REACHABLE: &str = "The other device is not reachable right now";

/// What can go through a running session's mailbox.
#[derive(Debug)]
pub enum SessionCommand {
    /// A frame for the peer.
    Forward(PeerFrame),
    /// End this session now, without a frame: its channel was switched off
    /// or removed (ADR-0008 D15).
    Close,
}

pub type SessionSender = mpsc::Sender<SessionCommand>;

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncStatus {
    pub peer_device_id: Option<String>,
    pub mapped_space_ids: Vec<String>,
    pub last_synced_at: Option<String>,
    pub last_synced_at_by_space: BTreeMap<String, Option<String>>,
    pub end_of_sync_at_by_space: BTreeMap<String, Option<String>>,
    pub pending_event_count: usize,
    pub outbox_event_count: i64,
    pub acked_event_count: i64,
    pub seen_event_count: i64,
    pub tombstone_count: i64,
}

/// One entry of the Device page's sync queue: a change made here that this
/// peer has not acked yet. `title` is given only for a quest and a space;
/// the frontend names the other entity types.
#[derive(Debug, Clone, Serialize)]
pub struct SyncQueueEntry {
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub title: Option<String>,
}

/// What the Device page shows about "has my data synced" for one peer.
/// `entries` is capped at `SYNC_QUEUE_ENTRY_LIMIT`; `pending_count` is the
/// real total.
#[derive(Debug, Clone, Serialize)]
pub struct SyncQueueSummary {
    pub peer_device_id: String,
    pub pending_count: usize,
    pub entries: Vec<SyncQueueEntry>,
    /// The newest ack from this peer.
    pub last_acked_at: Option<String>,
}

/// How many entries the sync queue shows.
pub const SYNC_QUEUE_ENTRY_LIMIT: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncTickPeer {
    pub peer_device_id: String,
    pub sent_events: usize,
    pub received_events: usize,
    pub acked_events: usize,
    pub transferred_objects: usize,
    pub last_transfer_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSyncTickResult {
    pub sent_events: usize,
    pub applied_events: usize,
    pub received_acks: usize,
    pub peers: Vec<SpaceSyncTickPeer>,
    pub ticked_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedCustomSpace {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceMappingApplyResult {
    pub mapped_space_ids: Vec<String>,
    pub unresolved_custom_spaces: Vec<UnresolvedCustomSpace>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceResolutionMode {
    CreateNew,
    UseExisting,
}

/// The sessions running now, the frames waiting for a session, and the
/// frames that arrived and wait for a tick.
#[derive(Default)]
struct Sessions {
    /// One session for each (peer, channel kind) at most (ADR-0008 D10).
    running: HashMap<(String, ChannelKind), SessionSender>,
    /// Frames that wait for the next session with each peer.
    pending_frames: HashMap<String, Vec<PeerFrame>>,
    incoming_sync_events: HashMap<String, SyncEventEnvelope>,
    incoming_sync_acks: HashMap<String, IncomingSyncAck>,
    incoming_space_mapping_updates: HashMap<String, IncomingSpaceMappingUpdate>,
    incoming_space_sync_ends: HashMap<String, IncomingSpaceSyncEnd>,
}

pub struct SpaceSyncService {
    identity: Identity,
    db_path: PathBuf,
    bus: Arc<EventBusService>,
    network: Arc<NetworkChannelService>,
    bluetooth: Arc<BluetoothChannelService>,
    /// This object, for the tasks it starts.
    me: Weak<SpaceSyncService>,
    /// Raised when there is sync work, in either direction. `notify_one`: a
    /// wake raised while the keeper ticks waits until it next waits.
    wake: tokio::sync::Notify,
    /// When the last tick ran. `None` until the first.
    last_tick_at: Mutex<Option<Instant>>,
    /// When each change was last sent to each device, keyed by
    /// `(peer_device_id, event_id)`.
    sent_at: Mutex<HashMap<(String, String), Instant>>,
    /// For each paired device, the channels in the order a frame goes over
    /// them: the primary first (ADR-0007).
    channel_order: Mutex<HashMap<String, [ChannelKind; 2]>>,
    keeper_started: Once,
    /// Set when the webview starts to load its page. On Android the
    /// Activity context that the Bluetooth adapter and the foreground
    /// service need exists only from then on.
    activity_up: std::sync::atomic::AtomicBool,
    sessions: Mutex<Sessions>,
    /// Bumped when the control outbox keeps a frame, so that a running
    /// session sends it now.
    outbox_kept: tokio::sync::watch::Sender<u64>,
}

impl SpaceSyncService {
    pub fn new(
        identity: Identity,
        db_path: PathBuf,
        bus: Arc<EventBusService>,
        network: Arc<NetworkChannelService>,
        bluetooth: Arc<BluetoothChannelService>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            identity,
            db_path,
            bus,
            network,
            bluetooth,
            me: me.clone(),
            wake: tokio::sync::Notify::new(),
            last_tick_at: Mutex::new(None),
            sent_at: Mutex::new(HashMap::new()),
            channel_order: Mutex::new(HashMap::new()),
            keeper_started: Once::new(),
            activity_up: std::sync::atomic::AtomicBool::new(false),
            sessions: Mutex::new(Sessions::default()),
            outbox_kept: tokio::sync::watch::channel(0).0,
        })
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("SpaceSync lives as long as its tasks")
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    fn channel(&self, kind: ChannelKind) -> Arc<dyn ChannelService> {
        match kind {
            ChannelKind::Network => self.network.clone(),
            ChannelKind::Bluetooth => self.bluetooth.clone(),
        }
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut SqliteConnection) -> T) -> T {
        tokio::task::block_in_place(|| f(&mut open_db_at_path(&self.db_path)))
    }

    // ---- When sync runs ----

    /// There is sync work: run a tick soon. A missed wake costs time, never
    /// data.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Wakes once `after` has passed: work that is due later.
    pub fn wake_after(&self, after: Duration) {
        let space_sync = self.arc();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(after).await;
            space_sync.wake();
        });
    }

    /// The webview started to load its page: on Android the Activity is up.
    pub fn record_activity_up(&self) {
        self.activity_up.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether a tick may use what needs the Android Activity (the Bluetooth
    /// channel, the foreground service). Always true off Android.
    fn activity_ready(&self) -> bool {
        cfg!(not(target_os = "android")) || self.activity_up.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(target_os = "android")]
    fn tick_is_overdue(&self) -> bool {
        match self.last_tick_at.lock() {
            Ok(at) => at.is_none_or(|at| at.elapsed() >= TICK_INTERVAL),
            Err(_) => true,
        }
    }

    /// A frame was kept in the control outbox: a running session sends it
    /// now, otherwise one is asked for.
    pub fn outbox_kept(&self) {
        self.outbox_kept.send_modify(|count| *count = count.wrapping_add(1));
        self.wake();
    }

    /// Starts the keeper: the task that ticks when woken, and on Android
    /// also when no tick ran in `TICK_INTERVAL`. And the task that wakes on
    /// the events that make sync work possible. Started at app start, so
    /// held connections come up while the frontend still loads.
    pub fn start(&self) {
        self.keeper_started.call_once(|| {
            let space_sync = self.arc();
            tauri::async_runtime::spawn(async move { space_sync.run_keeper().await });
            let space_sync = self.arc();
            tauri::async_runtime::spawn(async move { space_sync.run_event_listener().await });
        });
    }

    async fn run_keeper(self: Arc<Self>) {
        loop {
            // Registered before the wait, so a wake raised during a tick is
            // kept.
            let woken = self.wake.notified();
            #[cfg(target_os = "android")]
            {
                tokio::select! {
                    _ = tokio::time::sleep(TICK_INTERVAL) => {
                        if !self.tick_is_overdue() {
                            continue;
                        }
                    }
                    _ = woken => {}
                }
            }
            #[cfg(not(target_os = "android"))]
            woken.await;
            tokio::time::sleep(KEEPER_SETTLE).await;
            // The tick is blocking (SQLite and the sessions it starts), so it
            // runs off the async workers.
            let space_sync = self.clone();
            match tokio::task::spawn_blocking(move || space_sync.tick()).await {
                Ok(Err(err)) => log::warn!("[space_sync][keeper] tick failed: {err}"),
                Err(err) => log::warn!("[space_sync][keeper] tick task failed: {err}"),
                Ok(Ok(_)) => {}
            }
        }
    }

    /// Wakes, and closes sessions, on the events of the other services.
    async fn run_event_listener(self: Arc<Self>) {
        let mut events = self.bus.subscribe();
        loop {
            match events.recv().await {
                Ok(event) => self.on_event(event),
                // Missed events: look at everything again.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    self.close_sessions_without_channel();
                    self.wake();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    fn on_event(&self, event: Event) {
        match event {
            // A pair or a channel changed: a session on a channel that is
            // not on any more ends, a kept frame (`ChannelUnlinked`) goes,
            // and the channel order may be different.
            Event::PairedDevicesChanged | Event::ChannelsChanged { .. } => {
                self.close_sessions_without_channel();
                self.outbox_kept();
            }
            Event::DeviceHeard { .. } | Event::ConnectChanged { .. } => self.wake(),
            _ => {}
        }
    }

    /// Ends every session whose pair or channel is not on any more.
    fn close_sessions_without_channel(&self) {
        let running: Vec<(String, ChannelKind)> = match self.sessions.lock() {
            Ok(sessions) => sessions.running.keys().cloned().collect(),
            Err(_) => return,
        };
        for (peer, kind) in running {
            let on = self.with_conn(|conn| ChannelRepository::new(conn).is_enabled(&peer, kind));
            if !on {
                log::info!("[space_sync] {peer} {kind:?}: the channel is not on any more; ending its session");
                self.close_session_on(&peer, kind);
            }
        }
    }

    // ---- Sessions ----

    /// Registers a session (ADR-0008 D10). At most one for each (peer,
    /// channel kind); `false` means one runs already. Frames that wait for
    /// the peer go first.
    fn try_claim_session(&self, peer_device_id: &str, kind: ChannelKind, sender: SessionSender) -> bool {
        let queued = {
            let Ok(mut sessions) = self.sessions.lock() else {
                return false;
            };
            let key = (peer_device_id.to_string(), kind);
            if sessions.running.contains_key(&key) {
                return false;
            }
            sessions.running.insert(key, sender.clone());
            sessions.pending_frames.remove(peer_device_id).unwrap_or_default()
        };
        // What the new mailbox cannot take waits, before anything queued
        // since, for the next session.
        let mut left_over = Vec::new();
        for frame in queued {
            if !left_over.is_empty() || sender.try_send(SessionCommand::Forward(frame.clone())).is_err() {
                left_over.push(frame);
            }
        }
        if !left_over.is_empty() {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.pending_frames.entry(peer_device_id.to_string()).or_default().splice(0..0, left_over);
            }
        }
        self.bus.publish(Event::SessionChanged {
            peer_device_id: peer_device_id.to_string(),
            kind,
            established: true,
        });
        // A session that comes up makes waiting work sendable.
        self.wake();
        true
    }

    fn release_session(&self, peer_device_id: &str, kind: ChannelKind) {
        let removed = match self.sessions.lock() {
            Ok(mut sessions) => sessions.running.remove(&(peer_device_id.to_string(), kind)).is_some(),
            Err(_) => return,
        };
        if self.has_queued_frames(peer_device_id) {
            self.wake();
        }
        if removed {
            self.bus.publish(Event::SessionChanged {
                peer_device_id: peer_device_id.to_string(),
                kind,
                established: false,
            });
        }
    }

    /// The channel of a session that runs with this peer now, if any --
    /// Network first when both do.
    pub fn active_session_channel(&self, peer_device_id: &str) -> Option<ChannelKind> {
        let sessions = self.sessions.lock().ok()?;
        [ChannelKind::Network, ChannelKind::Bluetooth]
            .into_iter()
            .find(|kind| sessions.running.contains_key(&(peer_device_id.to_string(), *kind)))
    }

    /// Whether a session runs with this peer on this channel kind.
    pub fn has_session_on(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        self.sessions.lock().is_ok_and(|sessions| sessions.running.contains_key(&(peer_device_id.to_string(), kind)))
    }

    /// Whether a session runs with this peer on any channel.
    pub fn has_any_session(&self, peer_device_id: &str) -> bool {
        self.sessions
            .lock()
            .is_ok_and(|sessions| sessions.running.keys().any(|(id, _)| id == peer_device_id))
    }

    /// Sends a frame over a running session with this peer, on the first
    /// channel in its order. `false` when none runs.
    pub fn push_to_peer(&self, peer_device_id: &str, frame: PeerFrame) -> bool {
        let order = self.channel_order_for(peer_device_id);
        let Ok(sessions) = self.sessions.lock() else {
            return false;
        };
        Self::forward(&sessions, peer_device_id, frame, order)
    }

    fn forward(sessions: &Sessions, peer_device_id: &str, frame: PeerFrame, order: [ChannelKind; 2]) -> bool {
        order
            .into_iter()
            .filter_map(|kind| sessions.running.get(&(peer_device_id.to_string(), kind)))
            .next()
            .is_some_and(|sender| sender.try_send(SessionCommand::Forward(frame)).is_ok())
    }

    /// Sends a frame now if a session runs, otherwise keeps it for the next
    /// one and asks for one. In memory only: what must survive a restart is
    /// in the outbox.
    pub fn queue_for_peer(&self, peer_device_id: &str, frame: PeerFrame) {
        let order = self.channel_order_for(peer_device_id);
        let sent = {
            let Ok(mut sessions) = self.sessions.lock() else {
                return;
            };
            let waiting = sessions.pending_frames.get(peer_device_id).is_some_and(|q| !q.is_empty());
            let sent = !waiting && Self::forward(&sessions, peer_device_id, frame.clone(), order);
            if !sent {
                sessions.pending_frames.entry(peer_device_id.to_string()).or_default().push(frame);
            }
            sent
        };
        if !sent {
            self.wake();
        }
    }

    fn has_queued_frames(&self, peer_device_id: &str) -> bool {
        self.sessions
            .lock()
            .is_ok_and(|sessions| sessions.pending_frames.get(peer_device_id).is_some_and(|q| !q.is_empty()))
    }

    /// Ends a running session on this channel. `false` if none ran. A full
    /// mailbox does not lose the close: it waits for the next free slot.
    pub fn close_session_on(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        let sender = match self.sessions.lock() {
            Ok(sessions) => match sessions.running.get(&(peer_device_id.to_string(), kind)) {
                Some(sender) => sender.clone(),
                None => return false,
            },
            Err(_) => return false,
        };
        match sender.try_send(SessionCommand::Close) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(close)) => {
                tauri::async_runtime::spawn(async move {
                    let _ = sender.send(close).await;
                });
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// For the test switch (debug builds): ends every session on this
    /// channel.
    #[cfg(feature = "devtools")]
    pub fn end_sessions_on(&self, kind: ChannelKind) {
        let peers: Vec<String> = match self.sessions.lock() {
            Ok(sessions) => sessions.running.keys().filter(|(_, k)| *k == kind).map(|(p, _)| p.clone()).collect(),
            Err(_) => return,
        };
        for peer in peers {
            self.close_session_on(&peer, kind);
        }
    }

    /// The space mapping requests that arrived, oldest first, once.
    pub fn take_space_mapping_updates(&self) -> Vec<IncomingSpaceMappingUpdate> {
        let Ok(mut sessions) = self.sessions.lock() else {
            return Vec::new();
        };
        let mut updates: Vec<IncomingSpaceMappingUpdate> =
            sessions.incoming_space_mapping_updates.drain().map(|(_, update)| update).collect();
        updates.sort_by(|a, b| a.sent_at.cmp(&b.sent_at));
        updates
    }

    /// How many space mapping requests wait.
    pub fn space_mapping_update_count(&self) -> usize {
        self.sessions.lock().map(|sessions| sessions.incoming_space_mapping_updates.len()).unwrap_or(0)
    }

    /// How many sessions run.
    pub fn session_count(&self) -> usize {
        self.sessions.lock().map(|sessions| sessions.running.len()).unwrap_or(0)
    }

    /// Runs the session on a link to `peer_device_id` over `kind`, until it
    /// ends. `Err(link)` gives the link back when one runs already. Returns
    /// the close code the peer ended the link with, if it did.
    async fn hold_session(
        &self,
        link: Box<dyn DataLink>,
        peer_device_id: &str,
        kind: ChannelKind,
        on_held: Box<dyn FnOnce() + Send + '_>,
    ) -> Result<Option<u32>, Box<dyn DataLink>> {
        let (tx, rx) = mpsc::channel(64);
        if !self.try_claim_session(peer_device_id, kind, tx) {
            return Err(link);
        }
        on_held();
        let close_code = self.run_session(link, rx, peer_device_id).await;
        // The peer has no row for this channel. Removed there, or its setup
        // not done there yet; the device service decides, because it knows
        // whether a setup is open here.
        if close_code == Some(CLOSE_CHANNEL_MISSING) {
            self.bus.publish(Event::ChannelMissingAtPeer { peer_device_id: peer_device_id.to_string(), kind });
        }
        Ok(close_code)
    }

    /// One session with a peer (ADR-0008 D10): both sides push what they
    /// have, each message is acked, and the session runs until its link
    /// fails or it is asked to close. Returns the close code the peer
    /// closed the link with, if it did.
    async fn run_session(
        &self,
        mut link: Box<dyn DataLink>,
        mut rx: mpsc::Receiver<SessionCommand>,
        peer_device_id: &str,
    ) -> Option<u32> {
        let mut close_code = None;
        let kind = link.kind();

        // Our own Bluetooth address, for the peer's diagnostics, once for
        // each Network session.
        if kind == ChannelKind::Network {
            if let Some(address) = crate::services::device_service::local_bluetooth_address().await {
                let _ = send_frame(link.as_mut(), &PeerFrame::BluetoothAddressUpdate { address }).await;
            }
        }

        // Frames kept for this peer (ADR-0008 D14) go first, and again
        // whenever more are kept while the session runs.
        let mut kept = self.outbox_kept.subscribe();
        kept.mark_changed();

        loop {
            tokio::select! {
                Ok(()) = kept.changed() => {
                    if !self.send_kept_frames(link.as_mut(), peer_device_id).await {
                        break;
                    }
                }
                inbound = recv_frame(link.as_mut()) => {
                    let frame = match inbound {
                        Some(Ok(frame)) => frame,
                        Some(Err(err)) => {
                            let from = link.peer_addr().unwrap_or_else(|| "?".to_string());
                            log::info!("[exchange] {peer_device_id} {kind:?} from {from}: link error: {err}");
                            break;
                        }
                        None => {
                            let why = link.close_reason().unwrap_or_default();
                            log::info!("[exchange] {peer_device_id} {kind:?}: peer closed the link {why}");
                            close_code = link.close_code();
                            break;
                        }
                    };
                    self.handle_inbound(frame, link.as_mut(), peer_device_id).await;
                }
                Some(command) = rx.recv() => {
                    match command {
                        SessionCommand::Forward(frame) => {
                            if send_frame(link.as_mut(), &frame).await.is_err() {
                                break;
                            }
                        }
                        SessionCommand::Close => {
                            log::info!("[exchange] {peer_device_id} {kind:?}: asked to close");
                            break;
                        }
                    }
                }
            }
        }

        self.release_session(peer_device_id, kind);
        // What this session left unfinished is still in the outbox; a tick
        // after a pause looks again. The channel is grey now, and the next
        // tick lets it connect again if the peer is still there.
        self.wake_after(EXCHANGE_END_RECHECK);
        self.wake_after(HELD_SESSION_RECONNECT);
        close_code
    }

    /// Writes every frame kept for the peer to the link, removing each once
    /// written. `false` if the link failed; what was not written stays kept.
    async fn send_kept_frames(&self, link: &mut dyn DataLink, peer_device_id: &str) -> bool {
        let waiting = self.with_conn(|conn| OutboxRepository::new(conn).waiting(peer_device_id));
        for (id, frame) in waiting {
            if send_frame(link, &frame).await.is_err() {
                return false;
            }
            self.with_conn(|conn| OutboxRepository::new(conn).sent(id));
        }
        true
    }

    async fn handle_inbound(&self, frame: PeerFrame, link: &mut dyn DataLink, peer_device_id: &str) {
        match frame {
            PeerFrame::SyncEvent(envelope) => {
                let event_id = envelope.event_id.clone();
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.incoming_sync_events.insert(envelope.event_id.clone(), envelope);
                }
                // Only a tick applies it.
                self.wake();
                let _ = send_frame(link, &PeerFrame::Ack { event_id }).await;
            }
            PeerFrame::Ack { event_id } => {
                let ack = IncomingSyncAck {
                    from_device_id: peer_device_id.to_string(),
                    event_id: event_id.clone(),
                    acked_at: utc_now(),
                };
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.incoming_sync_acks.insert(event_id, ack);
                }
                self.wake();
            }
            PeerFrame::SpaceMappingUpdate { mapped_space_ids, custom_spaces, sent_at } => {
                let first_space_id = mapped_space_ids.first().cloned().unwrap_or_else(|| "none".to_string());
                let key = format!("{peer_device_id}:{first_space_id}:{sent_at}");
                let update = IncomingSpaceMappingUpdate {
                    from_device_id: peer_device_id.to_string(),
                    mapped_space_ids,
                    custom_spaces,
                    sent_at,
                };
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.incoming_space_mapping_updates.insert(key, update);
                }
                // The frontend takes it.
                self.bus.publish(Event::SpaceMappingRequestsChanged);
            }
            PeerFrame::SpaceSyncEnd { space_id, ended_at } => {
                let key = format!("{peer_device_id}:{space_id}");
                let end = IncomingSpaceSyncEnd { from_device_id: peer_device_id.to_string(), space_id, ended_at };
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.incoming_space_sync_ends.insert(key, end);
                }
                self.wake();
            }
            PeerFrame::BootstrapStart { space_id } => {
                let events =
                    self.with_conn(|conn| OutboxRepository::new(conn).load_events_for_space(&space_id).unwrap_or_default());
                for event in events {
                    if send_frame(link, &PeerFrame::SyncEvent(event)).await.is_err() {
                        return;
                    }
                }
                let _ = send_frame(link, &PeerFrame::BootstrapEnd { space_id, completed_at: utc_now() }).await;
            }
            PeerFrame::BootstrapEnd { space_id, completed_at } => {
                self.with_conn(|conn| {
                    let _ = SpaceMappingRepository::new(conn).set_last_synced(peer_device_id, &space_id, &completed_at);
                });
            }
            // ADR-0008 D14: the peer unlinked this channel: remove ours too.
            PeerFrame::ChannelUnlinked { kind } => {
                let removed = self.with_conn(|conn| ChannelRepository::new(conn).remove(peer_device_id, kind));
                match removed {
                    Ok(()) => {
                        log::info!("[session] {peer_device_id}: the peer unlinked {kind:?}; removed it here");
                        self.bus.publish(Event::ChannelsChanged { peer_device_id: peer_device_id.to_string() });
                    }
                    Err(err) => log::warn!("[session] {peer_device_id}: removing {kind:?} failed: {err}"),
                }
            }
            PeerFrame::BluetoothAddressUpdate { address } => {
                let Some(address) = crate::services::device_service::normalize_bluetooth_address(&address) else {
                    return;
                };
                // Diagnostics only, onto a channel that exists.
                self.with_conn(|conn| {
                    ChannelRepository::new(conn).note_address(peer_device_id, ChannelKind::Bluetooth, &address)
                });
            }
            // Frames before a session go to `DeviceService`, never here.
            PeerFrame::PairStart(_)
            | PeerFrame::PairAccept(_)
            | PeerFrame::PairComplete(_)
            | PeerFrame::IdentifyDevice { .. }
            | PeerFrame::DeviceIdentified { .. }
            | PeerFrame::AddChannel { .. }
            | PeerFrame::AddChannelAccept { .. }
            // A frame type this build does not know: skipped, so that the
            // session keeps running.
            | PeerFrame::Unknown => {}
        }
    }

    /// The checks of a sync connection a peer opened: who it is (the key
    /// TLS proved), whether the channel is on here, and whether it crosses
    /// this device's own connection. Then the session runs, or the
    /// connection closes with a code (`CLOSE_*`).
    async fn accept_session(&self, mut link: Box<dyn DataLink>) {
        let kind = link.kind();
        let from_addr = link.peer_addr().unwrap_or_default();
        let device_id =
            link.peer_key().and_then(|key| self.with_conn(|conn| DeviceRepository::new(conn).find_by_key(&key)));
        let Some(device_id) = device_id else {
            log::warn!("[space_sync][accept] {kind:?} session from {from_addr} rejected: its key is pinned for no pair");
            link.close(CLOSE_NOT_PAIRED, "not paired; pair the devices again");
            if kind == ChannelKind::Bluetooth {
                self.bluetooth.close_rejected_link(from_addr, None);
            }
            return;
        };
        log::info!("[space_sync][accept] {kind:?} session from {device_id} via {from_addr}");

        // Checked here too, not only on the connecting side: a peer whose
        // own copy of this channel is still on keeps connecting. An
        // unreadable row reads as off, never as missing.
        let channel = self.with_conn(|conn| ChannelRepository::new(conn).find(&device_id, kind));
        match channel {
            Some(channel) if channel.enabled => {}
            Some(_) => {
                log::warn!("[space_sync][accept] {kind:?} session from {device_id} rejected: the channel is off for this pair");
                link.close(CLOSE_CHANNEL_OFF, "the channel is off for this pair");
                if kind == ChannelKind::Bluetooth {
                    self.bluetooth.close_rejected_link(from_addr, Some(&device_id));
                }
                return;
            }
            None => {
                log::warn!("[space_sync][accept] {kind:?} session from {device_id} rejected: the channel is missing for this pair");
                link.close(CLOSE_CHANNEL_MISSING, "the channel is missing for this pair");
                if kind == ChannelKind::Bluetooth {
                    self.bluetooth.close_rejected_link(from_addr, Some(&device_id));
                }
                return;
            }
        }

        // Both devices connected at once: the connection from the smaller
        // device id is kept on both sides.
        let connecting = self.channel(kind).connecting(&device_id);
        if connecting && self.identity.device_id.as_str() < device_id.as_str() {
            log::info!("[space_sync][accept] {kind:?} session from {device_id} rejected: this device's connection is kept");
            link.close(CLOSE_CROSSED, "crossed connections: the other device's connection is kept");
            return;
        }

        // A held session ends only when its connection drops, so a peer
        // that connects again started again, and the old connection is
        // dead: the old session goes. Wait until it released its slot.
        if self.close_session_on(&device_id, kind) {
            log::info!("[space_sync][accept] {kind:?} {device_id} connected again; closing its held session");
            let until = tokio::time::Instant::now() + HELD_SESSION_RELEASE_WAIT;
            while self.has_session_on(&device_id, kind) && tokio::time::Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        let held = self
            .hold_session(
                link,
                &device_id,
                kind,
                Box::new(|| log::info!("[space_sync][accept] {kind:?} session with {device_id} accepted")),
            )
            .await;
        match held {
            Ok(_) if kind == ChannelKind::Bluetooth => self.bluetooth.close_links_after_session(&device_id),
            Ok(_) => {}
            Err(mut link) => {
                log::warn!("[space_sync][accept] {kind:?} session from {device_id} rejected: a session is already active");
                link.close(CLOSE_SESSION_ACTIVE, "a session is already active on this channel");
            }
        }
    }

    // ---- Channel order ----

    /// Reads, for each paired device, which channel a frame goes over
    /// first (the primary, ADR-0007).
    fn record_channel_order(&self, conn: &mut SqliteConnection, peer_device_ids: &[String]) {
        let orders: Vec<(String, [ChannelKind; 2])> = peer_device_ids
            .iter()
            .map(|peer| (peer.clone(), Self::channel_order(ChannelRepository::new(conn).primary_kind(peer))))
            .collect();
        if let Ok(mut order) = self.channel_order.lock() {
            order.clear();
            order.extend(orders);
        }
    }

    /// The channels a frame for `peer_device_id` goes over, in order.
    pub fn channel_order_for(&self, peer_device_id: &str) -> [ChannelKind; 2] {
        self.channel_order
            .lock()
            .ok()
            .and_then(|order| order.get(peer_device_id).copied())
            .unwrap_or_else(|| Self::channel_order(None))
    }

    /// The primary first (ADR-0007), otherwise Network first.
    fn channel_order(primary: Option<ChannelKind>) -> [ChannelKind; 2] {
        match primary {
            Some(ChannelKind::Bluetooth) => [ChannelKind::Bluetooth, ChannelKind::Network],
            _ => [ChannelKind::Network, ChannelKind::Bluetooth],
        }
    }

    // ---- The tick ----

    /// Whether `event_id` may go to `peer_device_id` now: it did not go to
    /// that device in the last `EVENT_RESEND_COOLDOWN`.
    fn may_send(&self, peer_device_id: &str, event_id: &str, now: Instant) -> bool {
        let sent_at = self.sent_at.lock().unwrap();
        sent_at
            .get(&(peer_device_id.to_string(), event_id.to_string()))
            .is_none_or(|&last| now.duration_since(last) >= EVENT_RESEND_COOLDOWN)
    }

    fn note_sent(&self, peer_device_id: &str, event_id: &str, now: Instant) {
        self.sent_at.lock().unwrap().insert((peer_device_id.to_string(), event_id.to_string()), now);
    }

    /// One pass of sync: keep the channels serving, send what waits, apply
    /// what arrived, record the acks.
    pub fn tick(&self) -> Result<SpaceSyncTickResult, String> {
        let mut conn = open_db_at_path(&self.db_path);
        let conn = &mut conn;
        if let Ok(mut at) = self.last_tick_at.lock() {
            *at = Some(Instant::now());
        }
        let peer_ids = DeviceRepository::new(conn).ids();
        self.record_channel_order(conn, &peer_ids);

        // Keep this device reachable and its held connections up. Before
        // the Activity is up, Network alone.
        let activity_ready = self.activity_ready();
        self.network.clone().keep_serving();
        if activity_ready {
            self.bluetooth.clone().keep_serving();
        }

        // The foreground service keeps the process alive on Android.
        #[cfg(target_os = "android")]
        if activity_ready {
            start_sync_service_once();
        }

        let ticked_at = utc_now();
        let mut transferred_by_peer: HashMap<String, (usize, usize, usize)> = HashMap::new();

        let ends: Vec<IncomingSpaceSyncEnd> = self
            .sessions
            .lock()
            .map(|mut sessions| sessions.incoming_space_sync_ends.drain().map(|(_, end)| end).collect())
            .unwrap_or_default();
        for end in ends {
            SpaceMappingRepository::new(conn).end(&end.from_device_id, &[end.space_id.clone()], &end.ended_at)?;
        }

        let mut sent_events_total = 0usize;
        for peer_device_id in &peer_ids {
            let mapped = SpaceMappingRepository::new(conn).active(peer_device_id)?;
            let pending = OutboxRepository::new(conn).load_unacked_events_for_peer(peer_device_id, &mapped)?;
            let mut sent_events = 0usize;
            for event in &pending {
                if sent_events >= MAX_EVENTS_PER_PEER_PER_TICK {
                    break;
                }
                // An unacked change stays pending until it is acked; without
                // this a peer slow to ack gets it again on every tick.
                let now = Instant::now();
                if !self.may_send(peer_device_id, &event.event_id, now) {
                    continue;
                }
                if self.push_to_peer(peer_device_id, PeerFrame::SyncEvent(event.clone())) {
                    self.note_sent(peer_device_id, &event.event_id, now);
                    sent_events += 1;
                }
            }
            sent_events_total += sent_events;
            if sent_events > 0 {
                transferred_by_peer.entry(peer_device_id.clone()).or_insert((0, 0, 0)).0 += sent_events;
            }
        }

        for peer_device_id in &peer_ids {
            let unsynced = SpaceMappingRepository::new(conn).never_synced(peer_device_id)?;
            if unsynced.is_empty() || !self.has_any_session(peer_device_id) {
                continue;
            }
            let mut all_taken = true;
            for space_id in unsynced {
                all_taken &= self.push_to_peer(peer_device_id, PeerFrame::BootstrapStart { space_id });
            }
            // A full mailbox did not take some; ask again soon.
            if !all_taken {
                self.wake_after(BOOTSTRAP_RETRY);
            }
        }

        let incoming: Vec<SyncEventEnvelope> = {
            let mut events: Vec<SyncEventEnvelope> = self
                .sessions
                .lock()
                .map(|mut sessions| sessions.incoming_sync_events.drain().map(|(_, event)| event).collect())
                .unwrap_or_default();
            events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.event_id.cmp(&b.event_id)));
            events
        };
        let mut applied_events = 0usize;
        let mut deferred = Vec::new();
        for event in incoming {
            if !SpaceMappingRepository::new(conn).is_active(&event.origin_device_id, &event.space_id).unwrap_or(false) {
                deferred.push(event);
                continue;
            }
            match apply::apply_sync_event(conn, &event) {
                Ok(applied) => {
                    if applied {
                        applied_events += 1;
                        transferred_by_peer.entry(event.origin_device_id.clone()).or_insert((0, 0, 0)).1 += 1;
                    }
                    // Queued if the session that carried the change closed,
                    // so that the peer stops sending it again.
                    self.queue_for_peer(&event.origin_device_id, PeerFrame::Ack { event_id: event.event_id.clone() });
                }
                Err(err) => log::warn!("[space-sync] applying incoming event {} failed: {err}", event.event_id),
            }
        }
        if !deferred.is_empty() {
            if let Ok(mut sessions) = self.sessions.lock() {
                for event in deferred {
                    sessions.incoming_sync_events.insert(event.event_id.clone(), event);
                }
            }
        }

        let acks: Vec<IncomingSyncAck> = self
            .sessions
            .lock()
            .map(|mut sessions| sessions.incoming_sync_acks.drain().map(|(_, ack)| ack).collect())
            .unwrap_or_default();
        let mut received_acks = 0usize;
        for ack in acks {
            if !OutboxRepository::new(conn).record_ack(&ack.from_device_id, &ack.event_id).unwrap_or(false) {
                continue;
            }
            if let Some((space_id, updated_at)) = OutboxRepository::new(conn).space_and_time(&ack.event_id)? {
                SpaceMappingRepository::new(conn).record_synced(&ack.from_device_id, &space_id, &updated_at)?;
            }
            received_acks += 1;
            transferred_by_peer.entry(ack.from_device_id.clone()).or_insert((0, 0, 0)).2 += 1;
        }

        let mut peers: Vec<SpaceSyncTickPeer> = transferred_by_peer
            .into_iter()
            .map(|(peer_device_id, (sent_events, received_events, acked_events))| SpaceSyncTickPeer {
                peer_device_id,
                sent_events,
                received_events,
                acked_events,
                transferred_objects: received_events + acked_events,
                last_transfer_at: ticked_at.clone(),
            })
            .collect();
        peers.sort_by(|a, b| a.peer_device_id.cmp(&b.peer_device_id));

        let _ = apply::cleanup_old_tombstones(conn);

        // Only when a peer's change landed here: the UI re-reads then, and
        // an idle pair makes no work for anyone.
        if applied_events > 0 {
            self.bus.publish(Event::SpaceDataChanged);
            // Applying can write changes of its own (a checklist merge that
            // converged); the next tick sends them.
            self.wake();
        }

        Ok(SpaceSyncTickResult { sent_events: sent_events_total, applied_events, received_acks, peers, ticked_at })
    }

    // ---- Space mappings ----

    /// The spaces this pair syncs now.
    pub fn list_mappings(&self, conn: &mut SqliteConnection, peer_device_id: &str) -> Result<Vec<String>, String> {
        SpaceMappingRepository::new(conn).active(peer_device_id)
    }

    /// Sets the spaces this pair syncs. A new space is a request that the
    /// other person answers at their device, so it needs the peer reachable
    /// now. An ended space is told to the peer through the outbox.
    pub fn update_mappings(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        mapped_space_ids: Vec<String>,
    ) -> Result<Vec<String>, String> {
        let before: BTreeSet<String> = SpaceMappingRepository::new(conn).active(peer_device_id)?.into_iter().collect();
        let desired = apply::normalized_space_ids(mapped_space_ids);
        let desired_set: BTreeSet<String> = desired.iter().cloned().collect();
        let requested: Vec<String> = desired_set.difference(&before).cloned().collect();
        let ended: Vec<String> = before.difference(&desired_set).cloned().collect();
        let ended_at = utc_now();

        if !requested.is_empty() && !self.peer_reachable_now(conn, peer_device_id) {
            return Err(PEER_NOT_REACHABLE.to_string());
        }

        // The ends are written with the mapping change, so a space never
        // stops syncing here while the peer is left without word of it
        // (ADR-0008 D14).
        let mut failed = None;
        let mapped = conn
            .transaction::<_, diesel::result::Error, _>(|conn| {
                let mapped = match apply::apply_mappings(conn, peer_device_id, desired) {
                    Ok(mapped) => mapped,
                    Err(reason) => {
                        failed = Some(reason);
                        return Err(diesel::result::Error::RollbackTransaction);
                    }
                };
                for space_id in &ended {
                    OutboxRepository::keep_in(
                        conn,
                        peer_device_id,
                        &PeerFrame::SpaceSyncEnd { space_id: space_id.clone(), ended_at: ended_at.clone() },
                    )?;
                }
                Ok(mapped)
            })
            .map_err(|err| failed.take().unwrap_or_else(|| err.to_string()))?;
        if !ended.is_empty() {
            self.outbox_kept();
        }

        let custom_spaces = apply::load_custom_space_descriptors(conn, &requested)?;
        for space_id in requested {
            let custom: Vec<CustomSpaceDescriptor> =
                custom_spaces.iter().filter(|space| space.space_id == space_id).cloned().collect();
            if let Err(err) = self.send_mapping_update(conn, peer_device_id, &[space_id], &custom) {
                log::warn!("[space-sync] telling the peer of a mapping request failed: {err}");
            }
        }
        Ok(mapped)
    }

    /// Accepts the spaces a peer asked to sync. Custom spaces that this
    /// device does not have come back unresolved, for the person to choose.
    pub fn apply_remote_mappings(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        mapped_space_ids: Vec<String>,
        custom_spaces: Vec<CustomSpaceDescriptor>,
    ) -> Result<SpaceMappingApplyResult, String> {
        let (resolved, unresolved_custom_spaces) =
            apply::partition_remote_mapped_spaces(conn, mapped_space_ids, custom_spaces)?;
        let mut desired = SpaceMappingRepository::new(conn).active(peer_device_id)?;
        for space_id in resolved {
            if !desired.contains(&space_id) {
                desired.push(space_id);
            }
        }
        let mapped_space_ids = apply::apply_mappings(conn, peer_device_id, desired)?;
        Ok(SpaceMappingApplyResult { mapped_space_ids, unresolved_custom_spaces })
    }

    /// The person chose how to map a custom space of the peer: a new space,
    /// or a space that exists here.
    pub fn resolve_custom_space_mapping(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        remote_space_id: String,
        remote_space_name: Option<String>,
        resolution_mode: SpaceResolutionMode,
        existing_space_id: Option<String>,
    ) -> Result<SpaceMappingApplyResult, String> {
        if apply::is_builtin_space_id(&remote_space_id) {
            return Err("remote custom-space resolution expects non built-in space id".to_string());
        }
        match resolution_mode {
            SpaceResolutionMode::CreateNew => {
                let name = remote_space_name
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| apply::placeholder_space_name(&remote_space_id));
                apply::create_space_with_id(conn, &remote_space_id, &name)?;
            }
            SpaceResolutionMode::UseExisting => {
                let local_space_id = existing_space_id
                    .as_deref()
                    .ok_or_else(|| "existing_space_id is required for use_existing".to_string())?;
                apply::merge_local_space_into_remote_id(conn, local_space_id, &remote_space_id, remote_space_name.as_deref())?;
            }
        }
        let mut mapped = SpaceMappingRepository::new(conn).active(peer_device_id)?;
        if !mapped.contains(&remote_space_id) {
            mapped.push(remote_space_id);
        }
        let mapped_space_ids = apply::apply_mappings(conn, peer_device_id, mapped)?;
        let custom_spaces = apply::load_custom_space_descriptors(conn, &mapped_space_ids)?;
        if let Err(err) = self.send_mapping_update(conn, peer_device_id, &mapped_space_ids, &custom_spaces) {
            log::warn!("[space-sync] telling the peer of a mapping resolve failed: {err}");
        }
        Ok(SpaceMappingApplyResult { mapped_space_ids, unresolved_custom_spaces: Vec::new() })
    }

    /// A request the other person answers at their device: it goes while
    /// the peer is reachable, and is not kept for later.
    fn send_mapping_update(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        mapped_space_ids: &[String],
        custom_spaces: &[CustomSpaceDescriptor],
    ) -> Result<(), String> {
        if !self.peer_reachable_now(conn, peer_device_id) {
            return Err(format!("{peer_device_id} is not reachable now"));
        }
        self.queue_for_peer(
            peer_device_id,
            PeerFrame::SpaceMappingUpdate {
                mapped_space_ids: mapped_space_ids.to_vec(),
                custom_spaces: custom_spaces.to_vec(),
                sent_at: utc_now(),
            },
        );
        Ok(())
    }

    /// Whether a session runs with the peer on a channel that is on.
    fn peer_reachable_now(&self, conn: &mut SqliteConnection, peer_device_id: &str) -> bool {
        [ChannelKind::Network, ChannelKind::Bluetooth].into_iter().any(|kind| {
            ChannelRepository::new(conn).is_enabled(peer_device_id, kind) && self.has_session_on(peer_device_id, kind)
        })
    }

    // ---- What the Device page reads ----

    pub fn status(&self, conn: &mut SqliteConnection, peer_device_id: Option<String>) -> Result<SpaceSyncStatus, String> {
        let outbox_event_count: i64 =
            sync_outbox::table.count().get_result(&mut *conn).map_err(|e| e.to_string())?;
        let seen_event_count: i64 = sync_seen::table.count().get_result(&mut *conn).map_err(|e| e.to_string())?;
        let tombstone_count: i64 = tombstones::table.count().get_result(&mut *conn).map_err(|e| e.to_string())?;

        let (mapped_space_ids, last_synced_at, last_synced_at_by_space, end_of_sync_at_by_space, pending_event_count, acked_event_count) =
            if let Some(peer_id) = peer_device_id.as_deref() {
                let rows = SpaceMappingRepository::new(conn).rows(peer_id)?;
                let mapped: Vec<String> =
                    rows.iter().filter(|(_, _, ended)| ended.is_none()).map(|(space, _, _)| space.clone()).collect();
                let by_space: BTreeMap<String, Option<String>> = rows
                    .iter()
                    .filter(|(_, _, ended)| ended.is_none())
                    .map(|(space, synced, _)| (space.clone(), synced.clone()))
                    .collect();
                let ended_by_space: BTreeMap<String, Option<String>> = rows
                    .into_iter()
                    .filter(|(_, _, ended)| ended.is_some())
                    .map(|(space, _, ended)| (space, ended))
                    .collect();
                let last_synced = by_space.values().flatten().max().cloned();
                let pending = OutboxRepository::new(conn).load_unacked_events_for_peer(peer_id, &mapped)?.len();
                let acked: i64 = sync_acks::table
                    .filter(sync_acks::peer_device_id.eq(peer_id))
                    .count()
                    .get_result(&mut *conn)
                    .map_err(|e| e.to_string())?;
                (mapped, last_synced, by_space, ended_by_space, pending, acked)
            } else {
                (Vec::new(), None, BTreeMap::new(), BTreeMap::new(), 0, 0)
            };

        Ok(SpaceSyncStatus {
            peer_device_id,
            mapped_space_ids,
            last_synced_at,
            last_synced_at_by_space,
            end_of_sync_at_by_space,
            pending_event_count,
            outbox_event_count,
            acked_event_count,
            seen_event_count,
            tombstone_count,
        })
    }

    pub fn queue_summary(&self, conn: &mut SqliteConnection, peer_device_id: String) -> Result<SyncQueueSummary, String> {
        use crate::schema::quests;
        let mapped = SpaceMappingRepository::new(conn).active(&peer_device_id)?;
        // The same query the sender uses, so the number on the page and the
        // changes the next tick sends are the same.
        let pending = OutboxRepository::new(conn).load_unacked_events_for_peer(&peer_device_id, &mapped)?;
        let pending_count = pending.len();
        // One entry for each entity, the oldest first.
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut entries: Vec<SyncQueueEntry> = Vec::new();
        for event in pending {
            if entries.len() >= SYNC_QUEUE_ENTRY_LIMIT {
                break;
            }
            if !seen.insert((event.entity_type.clone(), event.entity_id.clone())) {
                continue;
            }
            let title: Option<String> = match event.entity_type.as_str() {
                "quest" => quests::table
                    .find(&event.entity_id)
                    .select(quests::title)
                    .first(&mut *conn)
                    .optional()
                    .map_err(|e| e.to_string())?,
                "space" => spaces::table
                    .find(&event.entity_id)
                    .select(spaces::name)
                    .first(&mut *conn)
                    .optional()
                    .map_err(|e| e.to_string())?,
                _ => None,
            };
            entries.push(SyncQueueEntry {
                entity_type: event.entity_type,
                entity_id: event.entity_id,
                space_id: event.space_id,
                title,
            });
        }
        let last_acked_at: Option<String> = sync_acks::table
            .filter(sync_acks::peer_device_id.eq(&peer_device_id))
            .select(diesel::dsl::max(sync_acks::acked_at))
            .first(&mut *conn)
            .map_err(|e| e.to_string())?;
        Ok(SyncQueueSummary { peer_device_id, pending_count, entries, last_acked_at })
    }
}

#[async_trait]
impl SessionHandler for SpaceSyncService {
    fn has_session(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        self.has_session_on(peer_device_id, kind)
    }

    fn close_session(&self, peer_device_id: &str, kind: ChannelKind) -> bool {
        self.close_session_on(peer_device_id, kind)
    }

    async fn hold<'a>(
        &'a self,
        link: Box<dyn DataLink>,
        peer_device_id: &'a str,
        kind: ChannelKind,
        on_held: Box<dyn FnOnce() + Send + 'a>,
    ) -> Result<Option<u32>, Box<dyn DataLink>> {
        self.hold_session(link, peer_device_id, kind, on_held).await
    }

    async fn accept(&self, link: Box<dyn DataLink>) {
        self.accept_session(link).await;
    }
}

/// Starts the Android foreground service that keeps this process alive in
/// the background, once. Only with the Nearby Devices permission: Android
/// 14 and later need it for a `connectedDevice` service, and the start
/// would end the process without it. Without the permission the next tick
/// tries again.
#[cfg(target_os = "android")]
fn start_sync_service_once() {
    if !crate::services::ble_plugin::permission_granted() {
        return;
    }
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        crate::services::android_context::call_static_context_void("com.fini.app.SyncForegroundService", "start");
    });
}


/// Applying what a peer sent: changes to quests and spaces, and the
/// mapping of spaces.
mod apply {
use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use super::UnresolvedCustomSpace;
use crate::models::peer_frame::{CustomSpaceDescriptor, SyncEventEnvelope};
use crate::models::{ChecklistActivity, FocusHistoryEntry, Quest, QuestSeries, Space};
use crate::repositories::{OutboxRepository, SpaceMappingRepository};
use crate::schema::{checklist_activity, focus_history, spaces, sync_outbox, tombstones};
use crate::services::db::utc_now;
use crate::services::quest::QuestService;

pub(super) fn normalized_space_ids(mapped_space_ids: Vec<String>) -> Vec<String> {
    mapped_space_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn is_builtin_space_id(space_id: &str) -> bool {
    matches!(space_id, "1" | "2" | "3")
}

pub(super) fn placeholder_space_name(space_id: &str) -> String {
    let short = if space_id.len() > 8 {
        &space_id[..8]
    } else {
        space_id
    };
    format!("Shared {short}")
}

fn ensure_spaces_exist(conn: &mut SqliteConnection, desired_ids: &[String]) -> Result<(), String> {
    if desired_ids.is_empty() {
        return Ok(());
    }

    let existing_space_ids: Vec<String> = spaces::table
        .filter(spaces::id.eq_any(desired_ids))
        .select(spaces::id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let existing: BTreeSet<String> = existing_space_ids.into_iter().collect();
    let missing: Vec<String> = desired_ids
        .iter()
        .filter(|id| !existing.contains((*id).as_str()))
        .cloned()
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    let mut next_order = spaces::table
        .select(diesel::dsl::max(spaces::item_order))
        .first::<Option<i64>>(conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
        + 1;

    for space_id in missing {
        diesel::insert_or_ignore_into(spaces::table)
            .values((
                spaces::id.eq(&space_id),
                spaces::name.eq(placeholder_space_name(&space_id)),
                spaces::item_order.eq(next_order),
            ))
            .execute(conn)
            .map_err(|e| e.to_string())?;
        next_order += 1;
    }

    Ok(())
}

fn missing_space_ids(
    conn: &mut SqliteConnection,
    desired_ids: &[String],
) -> Result<Vec<String>, String> {
    if desired_ids.is_empty() {
        return Ok(Vec::new());
    }

    let existing_space_ids: Vec<String> = spaces::table
        .filter(spaces::id.eq_any(desired_ids))
        .select(spaces::id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let existing: BTreeSet<String> = existing_space_ids.into_iter().collect();
    Ok(desired_ids
        .iter()
        .filter(|id| !existing.contains((*id).as_str()))
        .cloned()
        .collect())
}

pub(super) fn load_custom_space_descriptors(
    conn: &mut SqliteConnection,
    mapped_space_ids: &[String],
) -> Result<Vec<CustomSpaceDescriptor>, String> {
    let custom_ids: Vec<String> = mapped_space_ids
        .iter()
        .filter(|id| !is_builtin_space_id(id))
        .cloned()
        .collect();

    if custom_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows: Vec<Space> = spaces::table
        .filter(spaces::id.eq_any(&custom_ids))
        .select(Space::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    let mut by_id = std::collections::HashMap::new();
    for row in rows {
        by_id.insert(row.id.clone(), row.name.clone());
    }

    Ok(custom_ids
        .into_iter()
        .filter_map(|space_id| {
            by_id.get(&space_id).map(|name| CustomSpaceDescriptor {
                space_id,
                name: name.clone(),
            })
        })
        .collect())
}

/// Sets the spaces this pair syncs to `mapped_space_ids`: new ones start,
/// missing ones end. Returns the spaces it syncs after that.
pub(super) fn apply_mappings(
    conn: &mut SqliteConnection,
    peer_device_id: &str,
    mapped_space_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if !crate::repositories::DeviceRepository::new(conn).is_paired(peer_device_id) {
        return Err("paired device not found".to_string());
    }
    let desired_ids = normalized_space_ids(mapped_space_ids);
    let missing = missing_space_ids(conn, &desired_ids)?;
    if !missing.is_empty() {
        return Err(format!("unknown space ids: {}", missing.join(", ")));
    }
    let existing: BTreeSet<String> = SpaceMappingRepository::new(conn).active(peer_device_id)?.into_iter().collect();
    let desired: BTreeSet<String> = desired_ids.iter().cloned().collect();
    let to_add: Vec<String> = desired.difference(&existing).cloned().collect();
    let to_remove: Vec<String> = existing.difference(&desired).cloned().collect();

    // Synced again before the peer heard that it had ended: the end no
    // longer holds, and arriving late it would undo this.
    for space_id in &to_add {
        OutboxRepository::new(conn).drop_space_sync_end(peer_device_id, space_id)?;
    }
    let mut mappings = SpaceMappingRepository::new(conn);
    mappings.end(peer_device_id, &to_remove, &utc_now())?;
    mappings.start(peer_device_id, &to_add)?;
    mappings.active(peer_device_id)
}


fn upsert_space(conn: &mut SqliteConnection, space: &Space) -> Result<(), String> {
    diesel::insert_into(spaces::table)
        .values((
            spaces::id.eq(&space.id),
            spaces::name.eq(&space.name),
            spaces::item_order.eq(space.item_order),
            spaces::created_at.eq(&space.created_at),
        ))
        .on_conflict(spaces::id)
        .do_update()
        .set((
            spaces::name.eq(&space.name),
            spaces::item_order.eq(space.item_order),
            spaces::created_at.eq(&space.created_at),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn upsert_quest(conn: &mut SqliteConnection, quest: &Quest) -> Result<(), String> {
    ensure_spaces_exist(conn, &[quest.space_id.clone()])?;
    QuestService::new(conn).sync_upsert_quest(quest)
}

fn upsert_quest_series(conn: &mut SqliteConnection, series: &QuestSeries) -> Result<(), String> {
    ensure_spaces_exist(conn, &[series.space_id.clone()])?;
    QuestService::new(conn).sync_upsert_series(series)
}

fn upsert_focus_history(
    conn: &mut SqliteConnection,
    focus: &FocusHistoryEntry,
) -> Result<(), String> {
    ensure_spaces_exist(conn, &[focus.space_id.clone()])?;

    diesel::insert_into(focus_history::table)
        .values((
            focus_history::id.eq(&focus.id),
            focus_history::quest_id.eq(&focus.quest_id),
            focus_history::space_id.eq(&focus.space_id),
            focus_history::trigger.eq(&focus.trigger),
            focus_history::created_at.eq(&focus.created_at),
        ))
        .on_conflict(focus_history::id)
        .do_update()
        .set((
            focus_history::quest_id.eq(&focus.quest_id),
            focus_history::space_id.eq(&focus.space_id),
            focus_history::trigger.eq(&focus.trigger),
            focus_history::created_at.eq(&focus.created_at),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_entity(conn: &mut SqliteConnection, event: &SyncEventEnvelope) -> Result<(), String> {
    diesel::insert_or_ignore_into(tombstones::table)
        .values((
            tombstones::entity_type.eq(&event.entity_type),
            tombstones::entity_id.eq(&event.entity_id),
            tombstones::space_id.eq(&event.space_id),
            tombstones::deleted_at.eq(utc_now()),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    match event.entity_type.as_str() {
        "space" => {
            diesel::delete(spaces::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        "quest" => {
            QuestService::new(conn).sync_delete_quest(&event.entity_id)?;
        }
        "quest_series" => {
            QuestService::new(conn).sync_delete_series(&event.entity_id)?;
        }
        "focus_history" => {
            diesel::delete(focus_history::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        "checklist_activity" => {
            diesel::delete(checklist_activity::table.find(&event.entity_id))
                .execute(conn)
                .map_err(|e| e.to_string())?;
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn cleanup_old_tombstones(conn: &mut SqliteConnection) -> Result<usize, String> {
    let cutoff = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::days(30))
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let deleted = diesel::delete(tombstones::table.filter(tombstones::deleted_at.lt(&cutoff)))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(deleted)
}

/// Our own device id, read straight from `settings` (mirrors
/// `pairing::runtime::DEVICE_ID_KEY`) so the checklist merge push-back can stamp an
/// accurate `origin_device_id` without needing app-state (`apply_sync_event` only has `conn`).
fn local_device_id(conn: &mut SqliteConnection) -> Option<String> {
    crate::services::settings::load_setting(conn, "device.id")
        .ok()
        .flatten()
}

fn emit_checklist_convergence_event(
    conn: &mut SqliteConnection,
    remote_origin_device_id: &str,
    entity_id: &str,
) -> Result<(), String> {
    if let Some(own_device_id) = local_device_id(conn) {
        if own_device_id != remote_origin_device_id {
            let refreshed = QuestService::new(conn).get(entity_id)?;
            let payload = serde_json::to_string(&refreshed).map_err(|e| e.to_string())?;
            OutboxRepository::new(conn).emit_sync_event(
                &own_device_id,
                "quest",
                entity_id,
                &refreshed.space_id,
                "upsert",
                Some(payload),
            )?;
        }
    }
    Ok(())
}

/// Merges an incoming checklist quest's `description` (task-list text) against local state
/// and persists the result, regardless of whether the incoming event wins the whole-entity LWW
/// comparison for the quest's other fields. Issue #128 explicitly forbids whole-quest LWW for
/// checklist item state — this is the retained-base 3-way merge from the storage spike
/// (`services::checklist::merge_3way`). Only called when the local quest is a checklist quest
/// (`is_checklist`); plain prose quests never enter this path and follow normal whole-entity LWW
/// on `description` like any other field. Pushes the merged result back out to the origin peer
/// when it differs from what they sent, so both sides converge. Returns the merged description
/// for the caller to fold into `upsert_quest`.
fn merge_and_persist_quest_checklist(
    conn: &mut SqliteConnection,
    entity_id: &str,
    incoming_description: Option<&str>,
) -> Result<(Option<String>, bool), String> {
    QuestService::new(conn).sync_merge_quest_checklist(entity_id, incoming_description)
}

pub(super) fn apply_sync_event(
    conn: &mut SqliteConnection,
    event: &SyncEventEnvelope,
) -> Result<bool, String> {
    if OutboxRepository::new(conn).is_event_seen(&event.event_id)? {
        return Ok(false);
    }

    match event.op_type.as_str() {
        "delete" => {
            delete_entity(conn, event)?;
        }
        "upsert" => {
            let whole_entity_wins =
                match OutboxRepository::new(conn).load_latest_event_for_entity(&event.entity_type, &event.entity_id)? {
                    Some(local_event) => event.wins_over(&local_event),
                    None => true,
                };

            let payload = event
                .payload
                .as_ref()
                .ok_or_else(|| "missing payload for upsert event".to_string())?;
            let payload = payload.as_str();

            // Checklist items get a per-item merge instead of following whole-quest LWW (#128),
            // so a locally-known checklist quest is handled before the generic win/lose gate
            // below. A quest that isn't (yet) known locally as a checklist quest — including a
            // brand-new quest arriving for the first time — falls through to the normal
            // whole-entity path; its `description` is treated like any other field.
            let local_is_checklist = if event.entity_type == "quest" {
                QuestService::new(conn).sync_quest_is_checklist(&event.entity_id)?
            } else {
                false
            };

            if local_is_checklist {
                let mut quest: Quest = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                let (merged_description, should_emit_convergence) =
                    merge_and_persist_quest_checklist(
                        conn,
                        &event.entity_id,
                        quest.description.as_deref(),
                    )?;

                if whole_entity_wins {
                    quest.description = merged_description;
                    quest.is_checklist = true;
                    upsert_quest(conn, &quest)?;
                    if should_emit_convergence {
                        emit_checklist_convergence_event(
                            conn,
                            &event.origin_device_id,
                            &event.entity_id,
                        )?;
                    }
                } else {
                    if should_emit_convergence {
                        emit_checklist_convergence_event(
                            conn,
                            &event.origin_device_id,
                            &event.entity_id,
                        )?;
                    }
                    OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
                    return Ok(false);
                }
            } else if !whole_entity_wins {
                OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
                return Ok(false);
            } else {
                match event.entity_type.as_str() {
                    "space" => {
                        let space: Space =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_space(conn, &space)?;
                    }
                    "quest" => {
                        // Not a (locally-known) checklist quest — normal whole-entity upsert,
                        // description included, exactly as before #128.
                        let quest: Quest =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_quest(conn, &quest)?;
                    }
                    "quest_series" => {
                        let series: QuestSeries =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_quest_series(conn, &series)?;
                    }
                    "focus_history" => {
                        let focus: FocusHistoryEntry =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        upsert_focus_history(conn, &focus)?;
                    }
                    "checklist_activity" => {
                        let activity: ChecklistActivity =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        diesel::insert_or_ignore_into(checklist_activity::table)
                            .values((
                                checklist_activity::id.eq(&activity.id),
                                checklist_activity::quest_id.eq(&activity.quest_id),
                                checklist_activity::kind.eq(&activity.kind),
                                checklist_activity::detail.eq(&activity.detail),
                                checklist_activity::created_at.eq(&activity.created_at),
                                checklist_activity::origin_device_id.eq(&activity.origin_device_id),
                            ))
                            .execute(conn)
                            .map_err(|e| e.to_string())?;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    OutboxRepository::new(conn).mark_event_seen(&event.event_id)?;
    SpaceMappingRepository::new(conn).record_synced(&event.origin_device_id, &event.space_id, &event.updated_at)?;
    Ok(true)
}

fn next_space_item_order(conn: &mut SqliteConnection) -> Result<i64, String> {
    let value = spaces::table
        .select(diesel::dsl::max(spaces::item_order))
        .first::<Option<i64>>(conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
        + 1;
    Ok(value)
}

pub(super) fn create_space_with_id(
    conn: &mut SqliteConnection,
    space_id: &str,
    name: &str,
) -> Result<(), String> {
    let order = next_space_item_order(conn)?;
    diesel::insert_or_ignore_into(spaces::table)
        .values((
            spaces::id.eq(space_id),
            spaces::name.eq(name),
            spaces::item_order.eq(order),
        ))
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn merge_local_space_into_remote_id(
    conn: &mut SqliteConnection,
    local_space_id: &str,
    remote_space_id: &str,
    remote_space_name: Option<&str>,
) -> Result<(), String> {
    if local_space_id == remote_space_id {
        return Ok(());
    }

    if is_builtin_space_id(local_space_id) {
        return Err("cannot merge built-in spaces into custom remote id".to_string());
    }

    let local_space: Space = spaces::table
        .find(local_space_id)
        .select(Space::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;

    let target_exists = spaces::table
        .find(remote_space_id)
        .count()
        .get_result::<i64>(conn)
        .map_err(|e| e.to_string())?
        > 0;

    if !target_exists {
        let name = remote_space_name
            .map(str::to_string)
            .unwrap_or_else(|| local_space.name.clone());
        create_space_with_id(conn, remote_space_id, &name)?;
    }

    QuestService::new(conn)
        .sync_move_quests_and_series_between_spaces(local_space_id, remote_space_id)?;

    diesel::update(focus_history::table.filter(focus_history::space_id.eq(local_space_id)))
        .set(focus_history::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    diesel::update(sync_outbox::table.filter(sync_outbox::space_id.eq(local_space_id)))
        .set(sync_outbox::space_id.eq(remote_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    SpaceMappingRepository::new(conn).move_space(local_space_id, remote_space_id)?;

    diesel::delete(spaces::table.find(local_space_id))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub(super) fn partition_remote_mapped_spaces(
    conn: &mut SqliteConnection,
    mapped_space_ids: Vec<String>,
    custom_spaces: Vec<CustomSpaceDescriptor>,
) -> Result<(Vec<String>, Vec<UnresolvedCustomSpace>), String> {
    let desired_ids = normalized_space_ids(mapped_space_ids);
    let missing = missing_space_ids(conn, &desired_ids)?;
    if missing.is_empty() {
        return Ok((desired_ids, Vec::new()));
    }

    let missing_set: BTreeSet<String> = missing.into_iter().collect();
    let mut custom_name_by_id = std::collections::HashMap::new();
    for item in custom_spaces {
        custom_name_by_id.insert(item.space_id, item.name);
    }

    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();

    for space_id in desired_ids {
        if !missing_set.contains(&space_id) {
            resolved.push(space_id);
            continue;
        }

        if is_builtin_space_id(&space_id) {
            ensure_spaces_exist(conn, std::slice::from_ref(&space_id))?;
            resolved.push(space_id);
            continue;
        }

        let name = custom_name_by_id
            .get(&space_id)
            .cloned()
            .unwrap_or_else(|| placeholder_space_name(&space_id));
        unresolved.push(UnresolvedCustomSpace { space_id, name });
    }

    Ok((resolved, unresolved))
}
}
