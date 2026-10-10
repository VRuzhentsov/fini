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

/// A message of the transport-neutral Fini peer protocol: pairing handshake
/// plus authenticated sync. Carried by whichever `Transport`/`DataLink` is
/// currently selected for a peer (see `crate::services::communication::channel`).
/// Bump whenever a new `PeerFrame` variant is introduced that must not be
/// *proactively* sent to a peer that might not understand it yet (unlike a
/// frame sent only in reply to something the peer itself sent first, which
/// proves they're already on a compatible build). `PeerFrame::Unknown`
/// alone only protects an updated build's own deserialization of frames
/// *it* receives -- it does nothing for an older, already-installed peer
/// receiving a frame kind its own `PeerFrame` enum predates. `Auth`/
/// `AuthOk` exchange this once per session so both sides know whether the
/// other actually supports version-gated frames before sending one; an
/// older peer's `Auth`/`AuthOk` simply omits the field (`#[serde(default)]`
/// -> `0`), which reads as "supports nothing past the original protocol."
pub const PROTOCOL_VERSION: u32 = 4;


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PeerFrame {
    #[serde(rename = "auth")]
    Auth {
        device_id: String,
        peer_device_id: String,
        #[serde(default)]
        protocol_version: u32,
    },
    #[serde(rename = "auth_ok")]
    AuthOk {
        #[serde(default)]
        protocol_version: u32,
    },
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
    /// (`channel::bluetooth`'s own scan-side filtering, so a stranger not in
    /// add-mode is never even connected to). BLE advertisements can't carry
    /// a device_id/hostname the way mDNS's `DiscoveryBeacon` does (payload
    /// too small alongside the service UUID), and `PairRequestPayload`
    /// itself requires `to_device_id` up front -- this is what lets a
    /// scanner learn it before attempting a real `PairRequest`. Untrusted,
    /// same as `PairRequest`/`PairAccept`/`PairComplete`: discovery
    /// metadata is never the trust boundary, Fini's own pairing handshake
    /// is (`specs/device-connect/README.md`).
    ///
    /// Carries the prober's own identity: of two devices in add-mode only
    /// one dials the other (the lower advertised fingerprint), so this one
    /// connection is how the probed device learns of the prober too.
    #[serde(rename = "discovery_hello")]
    DiscoveryHello { device_id: String, hostname: String, endpoint_id: String },
    /// Reply to `DiscoveryHello`, sent only if the receiver is currently in
    /// add-mode itself -- `specs/device-connect/README.md`: "Only devices
    /// in add-mode are pairing candidates." `endpoint_id` is the replier's
    /// iroh key: a pairing leg to it dials iroh by that key (ADR-0009).
    #[serde(rename = "discovery_hello_reply")]
    DiscoveryHelloReply { device_id: String, hostname: String, endpoint_id: String },
    /// ADR-0008 D1/D2: one half of a channel's init. Pre-auth, sent over a
    /// fresh link by a device running a setup search for a paired peer.
    /// Answered with `HelloAck` only if the receiver has this device paired
    /// *and* is running a setup search for it on this channel itself -- so
    /// an init needs both people searching, and a device that is not
    /// setting the channel up (switched off, unlinked, or simply not asked)
    /// says nothing. Not a trust boundary: it proves "you know a device id
    /// I have paired", the same as `DiscoveryHello` (see #184).
    #[serde(rename = "hello")]
    Hello { device_id: String },
    /// Reply to `Hello`; carries the replier's own device id.
    #[serde(rename = "hello_ack")]
    HelloAck { device_id: String },
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
        let frame = PeerFrame::AuthFail {
            reason: "private detail".to_string(),
        };
        assert_eq!(frame.wire_type(), "auth_fail");
        let hello = PeerFrame::DiscoveryHello {
            device_id: "d".to_string(),
            hostname: "h".to_string(),
            endpoint_id: "k".to_string(),
        };
        assert_eq!(hello.wire_type(), "discovery_hello");
    }
}
