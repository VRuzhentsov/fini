use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::services::communication::pairing::types::{
    PairAcceptPayload, PairCompletePayload, PairRequestPayload,
};
use crate::services::communication::pairing::CustomSpaceDescriptor;

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

/// What can be sent through a running exchange's mailbox (`run_session`'s
/// `rx`). `Forward` carries a frame to the peer; the other two come from the
/// app's channel commands alone, so the CLI build has no use for them.
#[derive(Debug)]
pub enum SessionCommand {
    Forward(PeerFrame),
    /// End this exchange now, without a frame: its channel was switched off
    /// (ADR-0008 D15).
    #[cfg(any(feature = "ui-plane", test))]
    Close,
}

pub type SessionSender = mpsc::Sender<SessionCommand>;

/// A message of the Fini peer protocol: the frames before a session
/// (pairing, adding a channel, identifying a device; `PEER_ALPN`) and the
/// frames of a sync session (`SYNC_ALPN`). Carried by whichever `DataLink`
/// is selected for a peer (see `crate::services::communication::channel`).
///
/// A new session frame that an older peer must not receive changes the
/// version in `SYNC_ALPN`: two builds on different versions then share no
/// ALPN and do not connect. `PeerFrame::Unknown` only protects a build's
/// own decoding of frames it does not know.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PeerFrame {
    /// Starts pairing: a device in Add Device asks another to pair.
    #[serde(rename = "pair_start")]
    PairStart(PairRequestPayload),
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
    /// Pre-auth, sent over a fresh Bluetooth connection when Pair is pressed
    /// on a device the Add Device list shows from its advertisement. A BLE
    /// advertisement cannot carry the device id, name and iroh key (too
    /// small next to the service UUID), and `PairRequestPayload` needs
    /// `to_device_id` -- this is how the sender learns them before
    /// `PairStart`. Untrusted, the same as `PairStart`/`PairAccept`/
    /// `PairComplete`: Fini's own pairing handshake is the trust boundary
    /// (`specs/device-connect/README.md`).
    ///
    /// Carries the sender's own identity too, so the receiver learns of
    /// the sender on the same connection.
    #[serde(rename = "identify_device")]
    IdentifyDevice { device_id: String, hostname: String, endpoint_id: String },
    /// Reply to `IdentifyDevice`, sent only if the receiver is in Add Device
    /// itself -- `specs/device-connect/README.md`: "Only devices in
    /// add-mode are pairing candidates." `endpoint_id` is the replier's
    /// iroh key: a pairing step to it connects by that key (ADR-0009).
    #[serde(rename = "device_identified")]
    DeviceIdentified { device_id: String, hostname: String, endpoint_id: String },
    /// ADR-0008 D1/D2: one half of a channel's init. Pre-auth, sent over a
    /// fresh link by a device running a setup search for a paired peer.
    /// Answered with `AddChannelAccept` only if the receiver has this
    /// device paired *and* is running a setup search for it on this
    /// channel itself -- so an init needs both people searching, and a
    /// device that is not setting the channel up (switched off, unlinked,
    /// or simply not asked) says nothing. Not a trust boundary: it proves
    /// "you know a device id I have paired", the same as `IdentifyDevice`
    /// (see #184).
    #[serde(rename = "add_channel")]
    AddChannel { device_id: String },
    /// Reply to `AddChannel`; carries the replier's own device id.
    #[serde(rename = "add_channel_accept")]
    AddChannelAccept { device_id: String },
    /// "I unlinked this channel on my side" (ADR-0008 D14), pushed like
    /// any other frame. The receiver removes its own row for it.
    #[serde(rename = "channel_unlinked")]
    ChannelUnlinked {
        kind: crate::services::communication::channel::ChannelKind,
    },
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

impl PeerFrame {
    /// The frame's wire `type` alone, without its payload -- safe to log,
    /// where the frame itself may carry quest data.
    #[cfg(any(feature = "ui-plane", test))]
    pub fn wire_type(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.get("type")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log line for an unexpected first frame names the frame by its
    /// wire type and nothing else: a payload can carry quest data.
    #[test]
    fn wire_type_names_the_frame_without_its_payload() {
        let identify = PeerFrame::IdentifyDevice {
            device_id: "d".to_string(),
            hostname: "h".to_string(),
            endpoint_id: "k".to_string(),
        };
        assert_eq!(identify.wire_type(), "identify_device");
    }
}
