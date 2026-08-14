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

/// Unified per-row status shape (ADR-0003 Phase 2), shared between the
/// Network and Bluetooth rows: each transport supplies its own condition
/// logic for which state applies (see `build_transport_statuses`), but the
/// UI only ever has to render these three cases, for either row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RowState {
    /// Local preconditions for this transport aren't met at all -- nothing
    /// to distrust or trust yet, it simply can't carry a session right now.
    /// `reason` is the transport-specific "why" shown to the user.
    Unconfigured { reason: String },
    /// Preconditions are met; not the transport actually carrying the live
    /// session right now. `reliable` distinguishes "no reason to distrust
    /// it" (true -- including a peer with zero attempts yet, the same bar
    /// `network_effectively_available` already used before this ADR) from
    /// "recent consecutive attempts have failed" (false).
    Configured { reliable: bool },
    /// A session is live on this specific transport right now, confirmed by
    /// `session_kind` -- trustworthy as of ADR-0003 Phase 1, which is what
    /// makes sure a silently-dead session doesn't linger here.
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportStatus {
    pub kind: TransportKind,
    /// What the automatic dial order would pick first on the *next*
    /// establishment attempt -- independent of `state`, which reports what
    /// *is* true right now. The two can legitimately disagree: sticky
    /// handoff (`transport::selection`) means a session already live on one
    /// transport is kept even if the other becomes preferred while it's
    /// still up.
    pub preferred: bool,
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
/// never report a `Configured`/`Live` Bluetooth row regardless of stored
/// metadata, or the Device view would promise a fallback that silently
/// cannot sync.
#[cfg(any(target_os = "linux", target_os = "android"))]
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const BLUETOOTH_ADAPTER_IMPLEMENTED: bool = false;

/// Everything `build_transport_statuses` needs, one field per condition it
/// checks. A named-field struct instead of positional bools deliberately:
/// this function used to take six, Phase 2 adds two more, and transposing
/// two same-typed bools at a call site is exactly the class of bug a
/// struct's field names catch that positional args don't.
#[derive(Debug, Clone, Copy)]
pub struct TransportStatusInputs {
    /// Raw discovery presence (`network_peer_available`) -- "is this
    /// peer's beacon reaching us right now."
    pub network_present: bool,
    /// `network_effectively_available`'s failure-history half: not stuck
    /// failing to actually connect. Kept separate from `network_present`
    /// because `Unconfigured` (no presence at all) and `Configured {
    /// reliable: false }` (presenced, but recently unreachable) are
    /// different states with different `reason`/no-`reason` shapes.
    pub network_reliable: bool,
    pub bluetooth_enabled: bool,
    pub bluetooth_has_metadata: bool,
    pub bluetooth_os_paired: bool,
    /// `bluetooth_effectively_reliable` -- Bluetooth's counterpart to
    /// `network_reliable`.
    pub bluetooth_reliable: bool,
    pub network_connected: bool,
    pub bluetooth_connected: bool,
}

pub fn build_transport_statuses(inputs: TransportStatusInputs) -> Vec<TransportStatus> {
    let TransportStatusInputs {
        network_present,
        network_reliable,
        bluetooth_enabled,
        bluetooth_has_metadata,
        bluetooth_os_paired,
        bluetooth_reliable,
        network_connected,
        bluetooth_connected,
    } = inputs;

    let network_unconfigured_reason = if network_present {
        None
    } else {
        Some("Unavailable".to_string())
    };
    let bluetooth_unconfigured_reason =
        bluetooth_unconfigured_reason(bluetooth_enabled, bluetooth_has_metadata, bluetooth_os_paired);

    // Mirrors what `select_dial_order`'s real callers actually check
    // (`network_effectively_available`/`bluetooth_effectively_reliable`),
    // not just raw presence/configuration -- so `preferred` reports what
    // the next establishment attempt would *actually* pick, not merely
    // what's nominally configured.
    let network_selectable = network_present && network_reliable;
    let bluetooth_selectable = bluetooth_unconfigured_reason.is_none() && bluetooth_reliable;

    vec![
        TransportStatus {
            kind: TransportKind::Network,
            preferred: network_selectable,
            state: row_state(network_unconfigured_reason, network_connected, network_reliable),
        },
        TransportStatus {
            kind: TransportKind::Bluetooth,
            preferred: !network_selectable && bluetooth_selectable,
            state: row_state(bluetooth_unconfigured_reason, bluetooth_connected, bluetooth_reliable),
        },
    ]
}

/// Shared shape for both rows: given whether (and why) a transport isn't
/// configured at all, whether it's carrying the live session, and whether
/// recent attempts on it have been reliable, decide which of the three
/// `RowState` cases applies. Transport-specific work stays in each
/// transport's own "why unconfigured" logic (`bluetooth_unconfigured_reason`,
/// inline for Network above) -- this function only knows the shared shape.
fn row_state(unconfigured_reason: Option<String>, connected: bool, reliable: bool) -> RowState {
    if let Some(reason) = unconfigured_reason {
        return RowState::Unconfigured { reason };
    }
    if connected {
        return RowState::Live;
    }
    RowState::Configured { reliable }
}

fn bluetooth_unconfigured_reason(enabled: bool, has_metadata: bool, os_paired: bool) -> Option<String> {
    if !BLUETOOTH_ADAPTER_IMPLEMENTED {
        return Some("Bluetooth transport not implemented yet".to_string());
    }
    if !enabled {
        return Some("Disabled for this Fini pair".to_string());
    }
    if !has_metadata {
        return Some("Enable after OS Bluetooth pairing to store reconnect metadata".to_string());
    }
    if !os_paired {
        return Some("OS Bluetooth pairing required".to_string());
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
            network_reliable: true,
            bluetooth_enabled: true,
            bluetooth_has_metadata: true,
            bluetooth_os_paired: true,
            bluetooth_reliable: true,
            network_connected: false,
            bluetooth_connected: false,
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
    fn network_is_preferred_when_present_and_reliable() {
        let statuses = build_transport_statuses(ready_inputs());
        assert!(find(&statuses, TransportKind::Network).preferred);

        let not_present = build_transport_statuses(TransportStatusInputs {
            network_present: false,
            ..ready_inputs()
        });
        assert!(!find(&not_present, TransportKind::Network).preferred);

        let unreliable = build_transport_statuses(TransportStatusInputs {
            network_reliable: false,
            ..ready_inputs()
        });
        assert!(
            !find(&unreliable, TransportKind::Network).preferred,
            "recently-unreliable network must not be preferred even while presenced"
        );
    }

    /// ADR-0003 Phase 2: `Live` reports the actual session per row,
    /// independent of `Configured`/reliability -- a row can be `Configured`
    /// but not `Live` (network reachable, no session established yet), or
    /// `Live` on one transport while the other reports `Configured` too
    /// (Bluetooth ready and bonded, but the live session happens to be
    /// running over network since network is always preferred when both
    /// are available).
    #[test]
    fn live_reflects_the_actual_session_independent_of_configuration() {
        let network_live = build_transport_statuses(TransportStatusInputs {
            network_connected: true,
            ..ready_inputs()
        });
        let network = find(&network_live, TransportKind::Network);
        let bluetooth = find(&network_live, TransportKind::Bluetooth);
        assert_eq!(network.state, RowState::Live);
        assert_eq!(
            bluetooth.state,
            RowState::Configured { reliable: true },
            "bluetooth stays Configured, not Live, while network carries the session"
        );

        let bluetooth_live = build_transport_statuses(TransportStatusInputs {
            bluetooth_connected: true,
            ..ready_inputs()
        });
        assert_eq!(
            find(&bluetooth_live, TransportKind::Bluetooth).state,
            RowState::Live
        );
    }

    /// No Bluetooth `Transport`/`Link` adapter is registered on this
    /// platform (see `BLUETOOTH_ADAPTER_IMPLEMENTED`'s doc comment), so a
    /// Bluetooth session can never actually establish there. Status must
    /// report `Unconfigured` regardless of how complete the stored
    /// enablement metadata is, or the Device view would promise a fallback
    /// that silently cannot sync. On Linux/Android, where the real adapter
    /// (`transport::ble`) is wired up, full metadata *should* report
    /// `Configured` -- see `bluetooth_is_configured_with_full_metadata_where_implemented`
    /// below.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn bluetooth_is_never_configured_without_a_registered_adapter() {
        for network_present in [true, false] {
            let statuses = build_transport_statuses(TransportStatusInputs {
                network_present,
                ..ready_inputs()
            });
            let bluetooth = find(&statuses, TransportKind::Bluetooth);
            assert!(
                matches!(bluetooth.state, RowState::Unconfigured { .. }),
                "bluetooth must report Unconfigured, got {:?}",
                bluetooth.state
            );
            assert!(!bluetooth.preferred, "bluetooth must not report preferred");
        }
    }

    /// Mirror of the above for platforms where `transport::ble` is a real,
    /// registered adapter (Linux, Android): full enablement metadata should
    /// now report Bluetooth `Configured`, and preferred exactly when
    /// network isn't selectable.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bluetooth_is_configured_with_full_metadata_where_implemented() {
        let fallback = build_transport_statuses(TransportStatusInputs {
            network_present: false,
            ..ready_inputs()
        });
        let bluetooth = find(&fallback, TransportKind::Bluetooth);
        assert_eq!(bluetooth.state, RowState::Configured { reliable: true });
        assert!(bluetooth.preferred, "bluetooth should be preferred when network is absent");

        let both = build_transport_statuses(ready_inputs());
        let bluetooth = find(&both, TransportKind::Bluetooth);
        assert_eq!(bluetooth.state, RowState::Configured { reliable: true });
        assert!(!bluetooth.preferred, "network should be preferred when both are available");
    }

    /// Incomplete metadata must still gate configuration even with a real
    /// adapter registered.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bluetooth_still_requires_full_metadata_where_implemented() {
        for inputs in [
            TransportStatusInputs {
                bluetooth_enabled: false,
                ..ready_inputs()
            },
            TransportStatusInputs {
                bluetooth_has_metadata: false,
                ..ready_inputs()
            },
            TransportStatusInputs {
                bluetooth_os_paired: false,
                ..ready_inputs()
            },
        ] {
            let statuses = build_transport_statuses(TransportStatusInputs {
                network_present: false,
                ..inputs
            });
            let bluetooth = find(&statuses, TransportKind::Bluetooth);
            assert!(
                matches!(bluetooth.state, RowState::Unconfigured { .. }),
                "incomplete metadata must report Unconfigured, got {:?}",
                bluetooth.state
            );
        }
    }

    /// ADR-0003 Phase 2's actual new behavior: a `Configured` transport
    /// with recent consecutive failures reports `reliable: false`
    /// (renders amber), distinct from both `Unconfigured` (gray) and a
    /// trustworthy `Configured`/`Live` (green).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn unreliable_configured_bluetooth_is_distinguishable_from_reliable() {
        let unreliable = build_transport_statuses(TransportStatusInputs {
            network_present: false,
            bluetooth_reliable: false,
            ..ready_inputs()
        });
        assert_eq!(
            find(&unreliable, TransportKind::Bluetooth).state,
            RowState::Configured { reliable: false }
        );

        let reliable = build_transport_statuses(TransportStatusInputs {
            network_present: false,
            ..ready_inputs()
        });
        assert_eq!(
            find(&reliable, TransportKind::Bluetooth).state,
            RowState::Configured { reliable: true }
        );
    }
}
