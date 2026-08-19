//! Peer connection lifecycle events.
//!
//! ADR-0003 revision: the sticky single-session handoff invariant this
//! module used to document and enforce ("selection only happens at session
//! establishment; a live session on any transport is kept until it drops")
//! no longer holds. Both Network and Bluetooth now dial/accept and stay
//! connected independently of each other -- see
//! `DeviceConnectionState::try_claim_session` (per-(peer, transport)
//! claiming) and `DeviceConnectionState::recompute_primary_locked` (which
//! of the connected transports carries real traffic). What's left here is
//! just the lifecycle event bus every adapter's session loop reports
//! through.

use crate::services::transport::TransportKind;

/// Fan-out lifecycle events for a peer connection. Consumed by the UI and
/// the connection manager; never carries sync payloads (those stay on the
/// per-peer session mailbox, see `space_sync::types::SessionSender`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    SessionEstablished {
        peer_device_id: String,
        kind: TransportKind,
    },
    SessionEnded {
        peer_device_id: String,
        kind: TransportKind,
    },
}

pub type LifecycleBus = tokio::sync::broadcast::Sender<LifecycleEvent>;

const LIFECYCLE_BUS_CAPACITY: usize = 64;

pub fn new_lifecycle_bus() -> LifecycleBus {
    let (tx, _rx) = tokio::sync::broadcast::channel(LIFECYCLE_BUS_CAPACITY);
    tx
}
