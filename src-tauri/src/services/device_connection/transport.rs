use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    #[default]
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
    /// Whether an authenticated session is live on *this specific*
    /// transport right now — distinct from `available`, which only reports
    /// whether the precondition for one is met. Network is preferred
    /// whenever both are available (ADR-0001's selection order), so a
    /// peer can be fully reachable while this is `false` on the Bluetooth
    /// row because the live session happens to be running over network
    /// instead — that is expected, not a bug. See
    /// `docs/adr/0002-bluetooth-address-exchange-live-status-and-ble-pairing.md`
    /// Phase 2.
    pub connected: bool,
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

/// `transport::ble` (BlueZ on Linux, GATT via `ble_gatt::backend::android`
/// on Android, both through `ble-gatt`) is wired up in `lib.rs`/
/// `space_sync::commands` on both platforms. Everywhere else, status must
/// never report `available`/`preferred` regardless of stored metadata, or
/// the Device view would promise a fallback that silently cannot sync.
#[cfg(any(target_os = "linux", target_os = "android"))]
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = false;

pub fn build_transport_statuses(
    network_available: bool,
    bluetooth_enabled: bool,
    bluetooth_has_metadata: bool,
    bluetooth_os_paired: bool,
    network_connected: bool,
    bluetooth_connected: bool,
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
            connected: network_connected,
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
            connected: bluetooth_connected,
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
        let both = build_transport_statuses(true, true, true, true, false, false);
        assert!(both
            .iter()
            .any(|status| status.kind == TransportKind::Network && status.preferred));

        let fallback = build_transport_statuses(false, true, true, true, false, false);
        assert!(!fallback
            .iter()
            .any(|status| status.kind == TransportKind::Network && status.preferred));
    }

    /// Phase 2 of ADR 0002: `connected` reports live session state per row,
    /// independent of `available`/`preferred` — a row can be available but
    /// not currently connected (network reachable, no session established
    /// yet), or connected on one transport while the other reports
    /// available too (Bluetooth configured and bonded, but the live
    /// session happens to be running over network since network is always
    /// preferred when both are available).
    #[test]
    fn connected_reflects_live_session_state_independent_of_availability() {
        let statuses = build_transport_statuses(true, true, true, true, true, false);
        let network = statuses
            .iter()
            .find(|status| status.kind == TransportKind::Network)
            .expect("network status row");
        let bluetooth = statuses
            .iter()
            .find(|status| status.kind == TransportKind::Bluetooth)
            .expect("bluetooth status row");
        assert!(network.connected, "network row should report the live session");
        assert!(bluetooth.available, "bluetooth stays available even while not the live transport");
        assert!(!bluetooth.connected, "bluetooth row must not claim a session it isn't carrying");

        let bluetooth_live = build_transport_statuses(true, true, true, true, false, true);
        let bluetooth = bluetooth_live
            .iter()
            .find(|status| status.kind == TransportKind::Bluetooth)
            .expect("bluetooth status row");
        assert!(bluetooth.connected, "bluetooth row should report a live Bluetooth session");
    }

    /// No Bluetooth `Transport`/`Link` adapter is registered on this platform
    /// (see `BLUETOOTH_ADAPTER_IMPLEMENTED`'s doc comment), so a Bluetooth
    /// session can never actually establish there. Status must report
    /// `available`/`preferred: false` regardless of how complete the stored
    /// enablement metadata is, or the Device view would promise a fallback
    /// that silently cannot sync. On Linux/Android, where the real adapter
    /// (`transport::ble`) is wired up, full metadata *should* report ready —
    /// see `bluetooth_is_available_with_full_metadata_where_implemented`
    /// below.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn bluetooth_is_never_reported_available_without_a_registered_adapter() {
        for (network_available, enabled, has_metadata, os_paired) in [
            (true, true, true, true),
            (false, true, true, true),
        ] {
            let statuses =
                build_transport_statuses(network_available, enabled, has_metadata, os_paired, false, false);
            let bluetooth = statuses
                .iter()
                .find(|status| status.kind == TransportKind::Bluetooth)
                .expect("bluetooth status row");
            assert!(!bluetooth.available, "bluetooth must not report available");
            assert!(!bluetooth.preferred, "bluetooth must not report preferred");
        }
    }

    /// Mirror of the above for platforms where `transport::ble` is a real,
    /// registered adapter (Linux, Android): full enablement metadata should
    /// now report Bluetooth ready, and preferred exactly when network is
    /// not.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bluetooth_is_available_with_full_metadata_where_implemented() {
        let fallback = build_transport_statuses(false, true, true, true, false, false);
        let bluetooth = fallback
            .iter()
            .find(|status| status.kind == TransportKind::Bluetooth)
            .expect("bluetooth status row");
        assert!(bluetooth.available, "bluetooth should report available");
        assert!(bluetooth.preferred, "bluetooth should be preferred when network is absent");

        let both = build_transport_statuses(true, true, true, true, false, false);
        let bluetooth = both
            .iter()
            .find(|status| status.kind == TransportKind::Bluetooth)
            .expect("bluetooth status row");
        assert!(bluetooth.available, "bluetooth should still report available");
        assert!(!bluetooth.preferred, "network should be preferred when both are available");
    }

    /// Incomplete metadata must still gate availability even with a real
    /// adapter registered.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bluetooth_still_requires_full_metadata_where_implemented() {
        for (enabled, has_metadata, os_paired) in
            [(false, true, true), (true, false, true), (true, true, false)]
        {
            let statuses =
                build_transport_statuses(false, enabled, has_metadata, os_paired, false, false);
            let bluetooth = statuses
                .iter()
                .find(|status| status.kind == TransportKind::Bluetooth)
                .expect("bluetooth status row");
            assert!(!bluetooth.available, "incomplete metadata must not report available");
        }
    }
}
