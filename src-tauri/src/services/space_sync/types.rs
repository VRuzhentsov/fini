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

pub type SessionSender = mpsc::Sender<WsMessage>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
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
}
