use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Network,
    Bluetooth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportEndpoint {
    pub peer_device_id: String,
    pub kind: TransportKind,
    pub address: String,
    pub ws_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothTransportMetadata {
    pub peer_device_id: String,
    pub address: String,
    pub enabled: bool,
    pub os_paired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportStatus {
    pub kind: TransportKind,
    pub enabled: bool,
    pub available: bool,
    pub preferred: bool,
    pub detail: String,
}

pub fn select_transport_endpoint(
    peer_device_id: &str,
    network_endpoint: Option<TransportEndpoint>,
    bluetooth: Option<BluetoothTransportMetadata>,
) -> Option<TransportEndpoint> {
    if let Some(endpoint) = network_endpoint {
        return Some(endpoint);
    }

    let bluetooth = bluetooth?;
    if !bluetooth.enabled || !bluetooth.os_paired || bluetooth.address.trim().is_empty() {
        return None;
    }

    Some(TransportEndpoint {
        peer_device_id: peer_device_id.to_string(),
        kind: TransportKind::Bluetooth,
        address: bluetooth.address,
        ws_port: 0,
    })
}

/// No real Bluetooth `Transport`/`Link` adapter is registered yet — this PR
/// only wires up `tcp_ws` and `sim` in `lib.rs` (see
/// `docs/adr/0001-transport-neutral-peer-protocol.md`); the real adapter is
/// a stacked follow-up PR. Until it lands, no Bluetooth session can ever be
/// established, so status must never report `available`/`preferred`
/// regardless of stored metadata — otherwise the Device view would promise
/// a fallback that silently cannot sync. Flip to `true` when the adapter
/// lands.
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = false;

pub fn build_transport_statuses(
    network_available: bool,
    bluetooth_enabled: bool,
    bluetooth_has_metadata: bool,
    bluetooth_os_paired: bool,
) -> Vec<TransportStatus> {
    let bluetooth_ready = BLUETOOTH_ADAPTER_IMPLEMENTED
        && bluetooth_enabled
        && bluetooth_has_metadata
        && bluetooth_os_paired;

    vec![
        TransportStatus {
            kind: TransportKind::Network,
            enabled: true,
            available: network_available,
            preferred: network_available,
            detail: if network_available {
                "Available"
            } else {
                "Unavailable"
            }
            .to_string(),
        },
        TransportStatus {
            kind: TransportKind::Bluetooth,
            enabled: bluetooth_enabled,
            available: bluetooth_ready,
            preferred: !network_available && bluetooth_ready,
            detail: bluetooth_status_detail(
                bluetooth_enabled,
                bluetooth_has_metadata,
                bluetooth_os_paired,
            ),
        },
    ]
}

fn bluetooth_status_detail(enabled: bool, has_metadata: bool, os_paired: bool) -> String {
    if !BLUETOOTH_ADAPTER_IMPLEMENTED {
        return "Bluetooth transport not implemented yet".to_string();
    }
    if !enabled {
        return "Disabled for this Fini pair".to_string();
    }
    if !has_metadata {
        return "Enable after OS Bluetooth pairing to store reconnect metadata".to_string();
    }
    if !os_paired {
        return "OS Bluetooth pairing required".to_string();
    }
    "Available for fallback".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_endpoint(peer_device_id: &str) -> TransportEndpoint {
        TransportEndpoint {
            peer_device_id: peer_device_id.to_string(),
            kind: TransportKind::Network,
            address: "192.168.1.10".to_string(),
            ws_port: 45455,
        }
    }

    fn bluetooth_metadata(peer_device_id: &str) -> BluetoothTransportMetadata {
        BluetoothTransportMetadata {
            peer_device_id: peer_device_id.to_string(),
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            enabled: true,
            os_paired: true,
        }
    }

    #[test]
    fn network_transport_is_preferred_when_both_are_available() {
        let selected = select_transport_endpoint(
            "peer-a",
            Some(network_endpoint("peer-a")),
            Some(bluetooth_metadata("peer-a")),
        )
        .expect("network should be selected");

        assert_eq!(selected.kind, TransportKind::Network);
        assert_eq!(selected.address, "192.168.1.10");
    }

    #[test]
    fn bluetooth_transport_is_fallback_when_network_is_absent() {
        let selected =
            select_transport_endpoint("peer-a", None, Some(bluetooth_metadata("peer-a")))
                .expect("bluetooth should be selected");

        assert_eq!(selected.kind, TransportKind::Bluetooth);
        assert_eq!(selected.address, "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn bluetooth_transport_requires_explicit_enablement_metadata_and_os_pairing() {
        for metadata in [
            BluetoothTransportMetadata {
                enabled: false,
                ..bluetooth_metadata("peer-a")
            },
            BluetoothTransportMetadata {
                address: "".to_string(),
                ..bluetooth_metadata("peer-a")
            },
            BluetoothTransportMetadata {
                os_paired: false,
                ..bluetooth_metadata("peer-a")
            },
        ] {
            assert_eq!(
                select_transport_endpoint("peer-a", None, Some(metadata)),
                None
            );
        }
    }

    #[test]
    fn status_marks_network_preferred_when_available() {
        let both = build_transport_statuses(true, true, true, true);
        assert!(both
            .iter()
            .any(|status| status.kind == TransportKind::Network && status.preferred));

        let fallback = build_transport_statuses(false, true, true, true);
        assert!(!fallback
            .iter()
            .any(|status| status.kind == TransportKind::Network && status.preferred));
    }

    /// No Bluetooth `Transport`/`Link` adapter is registered in this PR
    /// (`tcp_ws`/`sim` only — see `BLUETOOTH_ADAPTER_IMPLEMENTED`'s doc
    /// comment), so a Bluetooth session can never actually establish.
    /// Status must report `available`/`preferred: false` regardless of how
    /// complete the stored enablement metadata is, or the Device view would
    /// promise a fallback that silently cannot sync.
    #[test]
    fn bluetooth_is_never_reported_available_without_a_registered_adapter() {
        // Every combination that used to make the old (buggy) heuristic
        // report Bluetooth as available/preferred:
        for (network_available, enabled, has_metadata, os_paired) in [
            (true, true, true, true),
            (false, true, true, true),
        ] {
            let statuses =
                build_transport_statuses(network_available, enabled, has_metadata, os_paired);
            let bluetooth = statuses
                .iter()
                .find(|status| status.kind == TransportKind::Bluetooth)
                .expect("bluetooth status row");
            assert!(!bluetooth.available, "bluetooth must not report available");
            assert!(!bluetooth.preferred, "bluetooth must not report preferred");
        }
    }
}
