//! The messages of the Fini peer protocol, as they go on the wire.

use serde::{Deserialize, Serialize};

use crate::models::channel::ChannelKind;

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

impl SyncEventEnvelope {
    /// Whether this change wins over `existing`, a change to the same
    /// entity: the newer `updated_at` wins; on a tie the lower
    /// `origin_device_id`, then the lower `event_id`.
    pub fn wins_over(&self, existing: &SyncEventEnvelope) -> bool {
        match self.updated_at.cmp(&existing.updated_at) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => match self.origin_device_id.cmp(&existing.origin_device_id) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => self.event_id < existing.event_id,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSpaceDescriptor {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequestPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    #[serde(default)]
    pub from_discovery_port: Option<u16>,
    #[serde(default)]
    pub from_ws_port: Option<u16>,
    pub to_device_id: String,
    pub created_at: String,
    pub expires_at: String,
    /// The sender's iroh key. Self-reported, for pairing over a plain
    /// Bluetooth link that proves none; over the Network channel the key
    /// TLS proved wins (ADR-0009 D8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairAcceptPayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub code: String,
    pub from_device_id: String,
    pub to_device_id: String,
    pub accepted_at: String,
    /// The sender's iroh key; see `PairRequestPayload::from_endpoint_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCompletePayload {
    pub protocol: String,
    pub kind: String,
    pub request_id: String,
    pub from_device_id: String,
    pub from_hostname: String,
    pub to_device_id: String,
    pub paired_at: String,
    /// The completing peer's own Bluetooth address, if known, sent whatever
    /// channel carries this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluetooth_address: Option<String>,
    /// Reserved for a later key agreement. Unused today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_material: Option<crate::services::channel_service::encryption::KeyMaterial>,
    /// The sender's iroh key; see `PairRequestPayload::from_endpoint_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_endpoint_id: Option<String>,
}

/// A message of the Fini peer protocol: the frames before a session
/// (pairing, adding a channel, identifying a device; `PEER_ALPN`) and the
/// frames of a sync session (`SYNC_ALPN`).
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
    BootstrapEnd { space_id: String, completed_at: String },
    #[serde(rename = "space_mapping_update")]
    SpaceMappingUpdate {
        mapped_space_ids: Vec<String>,
        custom_spaces: Vec<CustomSpaceDescriptor>,
        sent_at: String,
    },
    #[serde(rename = "space_sync_end")]
    SpaceSyncEnd { space_id: String, ended_at: String },
    /// This device's own Bluetooth address, sent once on a Network session
    /// by a device that can read it (Linux).
    #[serde(rename = "bluetooth_address_update")]
    BluetoothAddressUpdate { address: String },
    /// Before pairing, over a fresh Bluetooth link: asks the device for its
    /// id, name and iroh key, which an advertisement is too small to carry.
    /// Untrusted; carries the sender's own identity too.
    #[serde(rename = "identify_device")]
    IdentifyDevice { device_id: String, hostname: String, endpoint_id: String },
    /// Reply to `IdentifyDevice`, sent only while the receiver is in Add
    /// Device itself.
    #[serde(rename = "device_identified")]
    DeviceIdentified { device_id: String, hostname: String, endpoint_id: String },
    /// One half of a channel's init (ADR-0008 D1, D2), sent by a device that
    /// runs a setup for a paired device. Answered with `AddChannelAccept`
    /// only while the receiver runs its own setup for the sender on this
    /// channel, or the channel exists there.
    #[serde(rename = "add_channel")]
    AddChannel { device_id: String },
    /// Reply to `AddChannel`; carries the replier's own device id.
    #[serde(rename = "add_channel_accept")]
    AddChannelAccept { device_id: String },
    /// "I unlinked this channel on my side" (ADR-0008 D14). The receiver
    /// removes its own row for it.
    #[serde(rename = "channel_unlinked")]
    ChannelUnlinked { kind: ChannelKind },
    /// Any `type` tag this build does not know: skipped, so that a newer
    /// peer's frame does not end the session.
    #[serde(other)]
    Unknown,
}

impl PeerFrame {
    /// The frame's wire `type` alone, without its payload -- safe to log,
    /// where the frame itself may carry quest data.
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

    fn change(event_id: &str, origin_device_id: &str, updated_at: &str) -> SyncEventEnvelope {
        SyncEventEnvelope {
            event_id: event_id.to_string(),
            correlation_id: "corr-1".to_string(),
            origin_device_id: origin_device_id.to_string(),
            entity_type: "quest".to_string(),
            entity_id: "q1".to_string(),
            space_id: "1".to_string(),
            op_type: "upsert".to_string(),
            payload: None,
            updated_at: updated_at.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn the_newer_change_wins_and_ties_go_to_the_lower_ids() {
        let t1 = "2026-03-01T00:00:00Z";
        let t2 = "2026-03-02T00:00:00Z";
        assert!(change("e1", "a", t2).wins_over(&change("e2", "b", t1)));
        assert!(!change("e1", "a", t1).wins_over(&change("e2", "b", t2)));
        assert!(change("e1", "aaa", t1).wins_over(&change("e2", "bbb", t1)));
        assert!(!change("e1", "bbb", t1).wins_over(&change("e2", "aaa", t1)));
        assert!(change("aaa", "d", t1).wins_over(&change("bbb", "d", t1)));
        assert!(!change("bbb", "d", t1).wins_over(&change("aaa", "d", t1)));
    }

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
