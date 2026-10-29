//! What changed in the app (ADR-0010 D8, D12). `EventBusService` carries
//! each one to the services that subscribe and to the UI (D9). An event
//! tells that one thing changed and carries its new state, so a subscriber
//! that missed events is correct again after the next one.

use serde::Serialize;

use crate::models::channel::ChannelKind;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    /// A session on one channel to one paired device started or ended: the
    /// channel row turns green or grey.
    #[serde(rename_all = "camelCase")]
    SessionChanged {
        peer_device_id: String,
        kind: ChannelKind,
        established: bool,
    },
    /// The channels of one paired device changed: added, removed, switched,
    /// or the primary changed.
    #[serde(rename_all = "camelCase")]
    ChannelsChanged { peer_device_id: String },
    /// The paired devices changed: a pair was made or removed.
    PairedDevicesChanged,
    /// The devices that Add Device can list changed.
    AvailableDevicesChanged,
    /// A pair request arrived, was answered, or ended.
    PairRequestsChanged,
    /// Quests, spaces or mappings changed because of a peer.
    SpaceDataChanged,
    /// A paired device asked to sync a space, or ended the sync of one.
    SpaceMappingRequestsChanged,
    /// Changes went to a paired device, or it acked changes: the sync queue
    /// and the time of the last sync changed.
    SyncQueueChanged,
    /// A channel heard a paired device: a connection to it can start now.
    #[serde(rename_all = "camelCase")]
    DeviceHeard { peer_device_id: String, kind: ChannelKind },
    /// A connection to a paired device failed or was rejected, or the wait
    /// after that ended: the work for it can go another way, or try again.
    #[serde(rename_all = "camelCase")]
    ConnectChanged { peer_device_id: String, kind: ChannelKind },
}
