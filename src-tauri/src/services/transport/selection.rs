//! Transport selection and the sticky single-session handoff invariant.
//!
//! Rule (from `specs/space-sync/README.md`): network is preferred when
//! available; Bluetooth (in this PR, played by the `Sim` adapter — see
//! `crate::services::transport::sim`) is a fallback only. Handoff is
//! **sticky**: selection only happens at session establishment. A live
//! session on any transport is kept until it drops; the *next*
//! establishment attempt re-applies network-first ordering. This makes
//! duplicated or lost sync events structurally impossible, because at most
//! one authenticated session can ever be live for a peer at once — see
//! `DeviceConnectionState::try_claim_session`.

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

/// Network-first dial order for establishing a new session with a peer.
/// Pure function: `network_available` and `fallback_available` are the
/// caller's current presence/enablement snapshot for that peer, one flag
/// per candidate transport role. Establishment-only — never called while a
/// session is already live (see the module-level sticky-handoff rule).
pub fn select_dial_order(network_available: bool, fallback_available: bool) -> Vec<TransportKind> {
    let mut order = Vec::new();
    if network_available {
        order.push(TransportKind::TcpWs);
    }
    if fallback_available {
        order.push(TransportKind::Sim);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_first_when_both_available() {
        assert_eq!(
            select_dial_order(true, true),
            vec![TransportKind::TcpWs, TransportKind::Sim]
        );
    }

    #[test]
    fn falls_back_when_network_unavailable() {
        assert_eq!(select_dial_order(false, true), vec![TransportKind::Sim]);
    }

    #[test]
    fn empty_when_neither_available() {
        assert_eq!(select_dial_order(false, false), Vec::<TransportKind>::new());
    }

    #[test]
    fn network_only_when_fallback_not_enabled() {
        assert_eq!(select_dial_order(true, false), vec![TransportKind::TcpWs]);
    }
}
