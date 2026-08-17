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

/// Machine-readable reason code for a transport row's current state.
/// ADR-0003 revision: replaces every free-text `reason: String` the row
/// shapes used to carry. The frontend looks each variant up in its own
/// code -> display-text map (`transportStatusText.ts`) to render the "i"
/// icon's tooltip -- the variant name (its serialized `code` tag) is the
/// stable key a future locale file keys translations off of; nothing here
/// is meant to be shown to a user directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum TransportStatusCode {
    /// Network row, `Unconfigured`: no discovery presence for this peer.
    NetworkUnavailable,
    /// Bluetooth row, `Unconfigured`: no adapter registered on this
    /// platform at all (`BLUETOOTH_ADAPTER_IMPLEMENTED`).
    BluetoothNotSupported,
    /// Bluetooth row, `Unconfigured`: disabled for this pair.
    BluetoothDisabled,
    /// Bluetooth row, `Unconfigured`: enabled, but no address/reconnect
    /// metadata stored yet.
    BluetoothNoAddress,
    /// Bluetooth row, `Unconfigured`: has metadata, but the OS isn't
    /// currently bonded to it.
    BluetoothNotOsPaired,
    /// `Configured`, amber: a session is claimed on this transport but the
    /// bidirectional ping/ack proof hasn't completed even once yet.
    AwaitingFirstAck,
    /// `Configured`, amber: the bidirectional ping/ack proof was complete
    /// at some point but has since lapsed -- `count` is the number of
    /// consecutive missed cycles on whichever side (own outbound or the
    /// peer's inbound) is currently behind. See `TransportAckState`.
    PingMissed { count: u32 },
}

/// Unified per-row status shape (ADR-0003 revision): each transport
/// supplies its own condition logic for which state applies (see
/// `build_transport_statuses`), but the UI only ever has to render these
/// two cases, for either row. There is no separate "Live" case any more --
/// with both transports potentially connected and green at once, "which
/// one is carrying real traffic" is `TransportStatus::primary`, orthogonal
/// to a row's own gray/amber/green state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RowState {
    /// Gray: local preconditions for this transport aren't met at all --
    /// nothing to prove yet, it simply can't carry a session right now.
    Unconfigured { code: TransportStatusCode },
    /// Amber (`code: Some`) or green (`code: None`): preconditions are met
    /// and a session is claimed on this transport. Green requires the
    /// bidirectional ping/ack proof to be currently complete
    /// (`DeviceConnectionState::transport_reliable`) -- "continuously
    /// re-proven," not sticky, so a lapsed proof falls back to amber on its
    /// own without the transport having disconnected.
    Configured { code: Option<TransportStatusCode> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportStatus {
    pub kind: TransportKind,
    /// Whether this is the transport currently carrying real application
    /// traffic (SyncEvent, BootstrapStart, etc.) -- `DeviceConnectionState::
    /// primary_transport`. Network wins whenever it's connected, unless
    /// explicitly pinned to Bluetooth (recomputed on every connect/
    /// disconnect and on every manual pin change, so this always reflects
    /// what the automatic rule would pick right now -- there's no separate
    /// "would prefer" vs "is" distinction any more, since both transports
    /// stay independently connected rather than one waiting to take over).
    pub primary: bool,
    pub state: RowState,
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
/// never report a `Configured` Bluetooth row regardless of stored metadata,
/// or the Device view would promise a fallback that silently cannot sync.
#[cfg(any(target_os = "linux", target_os = "android"))]
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = false;

/// Everything `build_transport_statuses` needs, one field per condition it
/// checks. A named-field struct instead of positional bools deliberately:
/// transposing two same-typed bools at a call site is exactly the class of
/// bug a struct's field names catch that positional args don't.
#[derive(Debug, Clone, Copy)]
pub struct TransportStatusInputs {
    /// Raw discovery presence (`network_peer_available`) -- "is this
    /// peer's beacon reaching us right now."
    pub network_present: bool,
    pub bluetooth_enabled: bool,
    pub bluetooth_has_metadata: bool,
    pub bluetooth_os_paired: bool,
    /// Whether a session is currently claimed on this transport --
    /// `DeviceConnectionState::has_session_on`.
    pub network_connected: bool,
    pub bluetooth_connected: bool,
    /// `DeviceConnectionState::primary_transport`, resolved by the caller.
    pub network_primary: bool,
    pub bluetooth_primary: bool,
    /// `DeviceConnectionState::transport_liveness_code` -- the amber
    /// reason, or `None` for green. Only consulted when `*_connected` is
    /// true.
    pub network_code: Option<TransportStatusCode>,
    pub bluetooth_code: Option<TransportStatusCode>,
}

pub fn build_transport_statuses(inputs: TransportStatusInputs) -> Vec<TransportStatus> {
    let TransportStatusInputs {
        network_present,
        bluetooth_enabled,
        bluetooth_has_metadata,
        bluetooth_os_paired,
        network_connected,
        bluetooth_connected,
        network_primary,
        bluetooth_primary,
        network_code,
        bluetooth_code,
    } = inputs;

    let network_unconfigured_code = if network_present {
        None
    } else {
        Some(TransportStatusCode::NetworkUnavailable)
    };
    let bluetooth_unconfigured_code =
        bluetooth_unconfigured_code(bluetooth_enabled, bluetooth_has_metadata, bluetooth_os_paired);

    vec![
        TransportStatus {
            kind: TransportKind::Network,
            primary: network_primary,
            state: row_state(network_unconfigured_code, network_connected, network_code),
        },
        TransportStatus {
            kind: TransportKind::Bluetooth,
            primary: bluetooth_primary,
            state: row_state(bluetooth_unconfigured_code, bluetooth_connected, bluetooth_code),
        },
    ]
}

/// Shared shape for both rows: given whether (and why) a transport isn't
/// configured at all, and if it is, whether a session is claimed and what
/// amber code (if any) applies, decide which `RowState` case applies.
/// Transport-specific work stays in each transport's own "why unconfigured"
/// logic (`bluetooth_unconfigured_code`, inline for Network above) -- this
/// function only knows the shared shape.
fn row_state(
    unconfigured_code: Option<TransportStatusCode>,
    connected: bool,
    code: Option<TransportStatusCode>,
) -> RowState {
    if let Some(code) = unconfigured_code {
        return RowState::Unconfigured { code };
    }
    if !connected {
        return RowState::Configured {
            code: Some(TransportStatusCode::AwaitingFirstAck),
        };
    }
    RowState::Configured { code }
}

fn bluetooth_unconfigured_code(
    enabled: bool,
    has_metadata: bool,
    os_paired: bool,
) -> Option<TransportStatusCode> {
    if !BLUETOOTH_ADAPTER_IMPLEMENTED {
        return Some(TransportStatusCode::BluetoothNotSupported);
    }
    if !enabled {
        return Some(TransportStatusCode::BluetoothDisabled);
    }
    if !has_metadata {
        return Some(TransportStatusCode::BluetoothNoAddress);
    }
    if !os_paired {
        return Some(TransportStatusCode::BluetoothNotOsPaired);
    }
    None
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

    /// All inputs "healthy and ready" by default -- individual tests
    /// override just the field(s) they're exercising.
    fn ready_inputs() -> TransportStatusInputs {
        TransportStatusInputs {
            network_present: true,
            bluetooth_enabled: true,
            bluetooth_has_metadata: true,
            bluetooth_os_paired: true,
            network_connected: false,
            bluetooth_connected: false,
            network_primary: false,
            bluetooth_primary: false,
            network_code: None,
            bluetooth_code: None,
        }
    }

    fn find(statuses: &[TransportStatus], kind: TransportKind) -> TransportStatus {
        statuses
            .iter()
            .find(|status| status.kind == kind)
            .cloned()
            .unwrap_or_else(|| panic!("no {kind:?} status row"))
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
    fn a_transport_with_no_session_reports_unconfigured_or_awaiting_first_ack() {
        let not_present = build_transport_statuses(TransportStatusInputs {
            network_present: false,
            ..ready_inputs()
        });
        assert_eq!(
            find(&not_present, TransportKind::Network).state,
            RowState::Unconfigured {
                code: TransportStatusCode::NetworkUnavailable
            }
        );

        // Present but not yet connected: not unconfigured (preconditions
        // are met), but not green either -- no ping/ack proof exists yet.
        let presenced_only = build_transport_statuses(ready_inputs());
        assert_eq!(
            find(&presenced_only, TransportKind::Network).state,
            RowState::Configured {
                code: Some(TransportStatusCode::AwaitingFirstAck)
            }
        );
    }

    /// ADR-0003 revision's core new behavior: a claimed session isn't green
    /// on its own -- the bidirectional ping/ack proof (surfaced via
    /// `network_code`/`bluetooth_code`) decides amber vs green, independent
    /// of `primary`. A transport can be green and not primary (both
    /// connected and reliable; only one carries real traffic).
    #[test]
    fn green_requires_a_completed_ack_proof_independent_of_primary() {
        let statuses = build_transport_statuses(TransportStatusInputs {
            network_connected: true,
            network_primary: true,
            network_code: None,
            bluetooth_connected: true,
            bluetooth_primary: false,
            bluetooth_code: None,
            ..ready_inputs()
        });
        let network = find(&statuses, TransportKind::Network);
        let bluetooth = find(&statuses, TransportKind::Bluetooth);
        assert_eq!(network.state, RowState::Configured { code: None });
        assert!(network.primary);
        assert_eq!(
            bluetooth.state,
            RowState::Configured { code: None },
            "bluetooth can be green while not primary"
        );
        assert!(!bluetooth.primary);
    }

    #[test]
    fn a_connected_transport_awaiting_or_missing_ack_reports_amber() {
        let awaiting = build_transport_statuses(TransportStatusInputs {
            network_connected: true,
            network_code: Some(TransportStatusCode::AwaitingFirstAck),
            ..ready_inputs()
        });
        assert_eq!(
            find(&awaiting, TransportKind::Network).state,
            RowState::Configured {
                code: Some(TransportStatusCode::AwaitingFirstAck)
            }
        );

        let lapsed = build_transport_statuses(TransportStatusInputs {
            network_connected: true,
            network_code: Some(TransportStatusCode::PingMissed { count: 2 }),
            ..ready_inputs()
        });
        assert_eq!(
            find(&lapsed, TransportKind::Network).state,
            RowState::Configured {
                code: Some(TransportStatusCode::PingMissed { count: 2 })
            }
        );
    }

    /// No Bluetooth `Transport`/`Link` adapter is registered on this
    /// platform (see `BLUETOOTH_ADAPTER_IMPLEMENTED`'s doc comment), so a
    /// Bluetooth session can never actually establish there. Status must
    /// report `Unconfigured` regardless of how complete the stored
    /// enablement metadata is.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn bluetooth_is_never_configured_without_a_registered_adapter() {
        for network_present in [true, false] {
            let statuses = build_transport_statuses(TransportStatusInputs {
                network_present,
                ..ready_inputs()
            });
            let bluetooth = find(&statuses, TransportKind::Bluetooth);
            assert_eq!(
                bluetooth.state,
                RowState::Unconfigured {
                    code: TransportStatusCode::BluetoothNotSupported
                }
            );
            assert!(!bluetooth.primary);
        }
    }

    /// Mirror of the above for platforms where `transport::ble` is a real,
    /// registered adapter (Linux, Android).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bluetooth_is_configured_with_full_metadata_where_implemented() {
        let statuses = build_transport_statuses(TransportStatusInputs {
            network_present: false,
            ..ready_inputs()
        });
        let bluetooth = find(&statuses, TransportKind::Bluetooth);
        assert_eq!(
            bluetooth.state,
            RowState::Configured {
                code: Some(TransportStatusCode::AwaitingFirstAck)
            }
        );
    }

    /// Incomplete metadata must still gate configuration even with a real
    /// adapter registered.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bluetooth_still_requires_full_metadata_where_implemented() {
        for (inputs, expected) in [
            (
                TransportStatusInputs {
                    bluetooth_enabled: false,
                    ..ready_inputs()
                },
                TransportStatusCode::BluetoothDisabled,
            ),
            (
                TransportStatusInputs {
                    bluetooth_has_metadata: false,
                    ..ready_inputs()
                },
                TransportStatusCode::BluetoothNoAddress,
            ),
            (
                TransportStatusInputs {
                    bluetooth_os_paired: false,
                    ..ready_inputs()
                },
                TransportStatusCode::BluetoothNotOsPaired,
            ),
        ] {
            let statuses = build_transport_statuses(inputs);
            let bluetooth = find(&statuses, TransportKind::Bluetooth);
            assert_eq!(bluetooth.state, RowState::Unconfigured { code: expected });
        }
    }
}
