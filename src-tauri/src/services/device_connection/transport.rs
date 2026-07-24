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

pub fn build_transport_statuses(
    network_available: bool,
    bluetooth_enabled: bool,
    bluetooth_has_metadata: bool,
    bluetooth_os_paired: bool,
) -> Vec<TransportStatus> {
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
            available: bluetooth_enabled && bluetooth_has_metadata && bluetooth_os_paired,
            preferred: !network_available
                && bluetooth_enabled
                && bluetooth_has_metadata
                && bluetooth_os_paired,
            detail: bluetooth_status_detail(
                bluetooth_enabled,
                bluetooth_has_metadata,
                bluetooth_os_paired,
            ),
        },
    ]
}

fn bluetooth_status_detail(enabled: bool, has_metadata: bool, os_paired: bool) -> String {
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
    fn status_marks_network_preferred_until_it_is_unavailable() {
        let both = build_transport_statuses(true, true, true, true);
        assert!(both
            .iter()
            .any(|status| status.kind == TransportKind::Network && status.preferred));
        assert!(both
            .iter()
            .any(|status| status.kind == TransportKind::Bluetooth
                && status.available
                && !status.preferred));

        let fallback = build_transport_statuses(false, true, true, true);
        assert!(fallback
            .iter()
            .any(|status| status.kind == TransportKind::Bluetooth && status.preferred));
    }
}
