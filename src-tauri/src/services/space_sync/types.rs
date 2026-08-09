use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::services::device_connection::types::{
    PairAcceptPayload, PairCompletePayload, PairRequestPayload,
};
use crate::services::device_connection::CustomSpaceDescriptor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEventEnvelope {
    pub event_id: String,
    pub correlation_id: String,
    pub origin_device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub space_id: String,
    pub op_type: String,
    pub payload: Option<String>,
    pub updated_at: String,
    pub created_at: String,
}

pub type SessionSender = mpsc::Sender<PeerFrame>;

/// A message of the transport-neutral Fini peer protocol: pairing handshake
/// plus authenticated sync. Carried by whichever `Transport`/`Link` is
/// currently selected for a peer (see `crate::services::transport`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PeerFrame {
    #[serde(rename = "auth")]
    Auth {
        device_id: String,
        peer_device_id: String,
    },
    #[serde(rename = "auth_ok")]
    AuthOk,
    #[serde(rename = "auth_fail")]
    AuthFail { reason: String },
    #[serde(rename = "pair_request")]
    PairRequest(PairRequestPayload),
    #[serde(rename = "pair_accept")]
    PairAccept(PairAcceptPayload),
    #[serde(rename = "pair_complete")]
    PairComplete(PairCompletePayload),
    #[serde(rename = "sync_event")]
    SyncEvent(SyncEventEnvelope),
    #[serde(rename = "ack")]
    Ack { event_id: String },
    #[serde(rename = "bootstrap_start")]
    BootstrapStart { space_id: String },
    #[serde(rename = "bootstrap_end")]
    BootstrapEnd {
        space_id: String,
        completed_at: String,
    },
    #[serde(rename = "space_mapping_update")]
    SpaceMappingUpdate {
        mapped_space_ids: Vec<String>,
        custom_spaces: Vec<CustomSpaceDescriptor>,
        sent_at: String,
    },
    #[serde(rename = "space_sync_end")]
    SpaceSyncEnd { space_id: String, ended_at: String },
    /// Sent once by whichever side of an authenticated *network* session can
    /// read its own real Bluetooth adapter address (Linux, via
    /// `bluetoothctl` — Android cannot: `BluetoothAdapter.getAddress()` has
    /// returned a dummy value since Android 6.0 for every normal app, no
    /// workaround exists). Lets the other side learn a usable Bluetooth
    /// fallback address without the user typing it in by hand. See
    /// `docs/adr/0002-bluetooth-address-exchange-live-status-and-ble-pairing.md`.
    #[serde(rename = "bluetooth_address_update")]
    BluetoothAddressUpdate { address: String },
    /// Pre-auth, sent by a scanner over a fresh BLE connection to a
    /// candidate whose advertisement already carried the add-mode flag
    /// (`transport::ble`'s own scan-side filtering, so a stranger not in
    /// add-mode is never even connected to). BLE advertisements can't carry
    /// a device_id/hostname the way mDNS's `DiscoveryBeacon` does (payload
    /// too small alongside the service UUID), and `PairRequestPayload`
    /// itself requires `to_device_id` up front -- this is what lets a
    /// scanner learn it before attempting a real `PairRequest`. Untrusted,
    /// same as `PairRequest`/`PairAccept`/`PairComplete`: discovery
    /// metadata is never the trust boundary, Fini's own pairing handshake
    /// is (`specs/device-connect/README.md`).
    #[serde(rename = "discovery_hello")]
    DiscoveryHello,
    /// Reply to `DiscoveryHello`, sent only if the receiver is currently in
    /// add-mode itself -- `specs/device-connect/README.md`: "Only devices
    /// in add-mode are pairing candidates."
    #[serde(rename = "discovery_hello_reply")]
    DiscoveryHelloReply { device_id: String, hostname: String },
    /// Catches any `type` tag this build doesn't recognize, instead of
    /// failing to decode outright. Without this, a peer running an older
    /// build that unconditionally receives a newer frame kind (e.g.
    /// `BluetoothAddressUpdate`, sent proactively into an already
    /// *authenticated* `run_session` loop) would hit a decode error on
    /// `recv_frame` -- and `run_session`'s `let Some(Ok(frame)) = inbound
    /// else { break }` treats that as fatal, silently ending the whole
    /// sync session rather than just skipping the one frame it didn't
    /// understand. Mixed-version paired devices (one side updated, one
    /// not) are the normal case during a rollout, not an edge case.
    /// `#[serde(other)]` must be a unit variant and is matched only when no
    /// named variant's tag matches.
    #[serde(other)]
    Unknown,
}
