use serde::{Deserialize, Serialize};

/// The channel kind, defined once in `channel` and re-exported here so the
/// many `pairing::ChannelKind` paths keep reading naturally. This module
/// used to declare a second enum of its own, with a `From` impl bridging
/// the two — see `ChannelKind`'s own doc comment for why that is gone.
pub use crate::services::communication::channel::ChannelKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelEndpoint {
    pub peer_device_id: String,
    pub kind: ChannelKind,
    pub address: String,
    pub ws_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothChannelMetadata {
    pub peer_device_id: String,
    pub address: String,
    pub enabled: bool,
    pub os_paired: bool,
}

/// Machine-readable reason code for a channel row's current state.
/// ADR-0003 revision: replaces every free-text `reason: String` the row
/// shapes used to carry. The frontend looks each variant up in its own
/// code -> display-text map (`channelStatusCodes.ts`) to render the "i"
/// icon's tooltip -- the variant name (its serialized `code` tag) is the
/// stable key a future locale file keys translations off of; nothing here
/// is meant to be shown to a user directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ChannelStatusCode {
    /// Network row, `Unconfigured`: no discovery presence for this peer.
    NetworkUnavailable,
    /// Network row, `Unconfigured`: switched off for this pair, the
    /// counterpart to `BluetoothDisabled`. Checked before presence, so a
    /// channel the user turned off says so rather than blaming the peer's
    /// network.
    NetworkDisabled,
    /// Bluetooth row, `Unconfigured`: no adapter registered on this
    /// platform at all -- `Radio::available` is false, so no build of this app
    /// on this OS could use it.
    BluetoothNotSupported,
    /// Bluetooth row, `Unconfigured`: disabled for this pair.
    BluetoothDisabled,
    /// Bluetooth row, `Unconfigured`: the channel is on for this pair, but
    /// the *local* radio refused the last time this process tried to use it
    /// -- Bluetooth switched off at the OS level, or an adapter that
    /// reports itself present and then declines to scan.
    ///
    /// This is the one `Unconfigured` code the user is expected to sit in
    /// deliberately: switching a channel on while the radio is off does not
    /// fail and does not snap the switch back, it parks here and starts by
    /// itself once the condition clears. See
    /// `ble::is_bluetooth_adapter_unavailable`.
    ///
    /// Ordered before `BluetoothPeerNotNearby` on purpose, and the ordering
    /// is load-bearing rather than cosmetic: with our own radio off we have
    /// not looked for the peer at all, so reporting "isn't nearby" would be
    /// a statement we have no evidence for, about the wrong device.
    BluetoothAdapterOff,
    /// Bluetooth row, `Unconfigured`: enabled, but no address/reconnect
    /// metadata stored yet.
    BluetoothNoAddress,
    /// Bluetooth row, `Unconfigured`: has metadata, but the OS isn't
    /// currently bonded to it.
    /// ADR-0006 slice 4. This slot used to be `BluetoothNotOsPaired`, and
    /// the reuse is deliberate: dropping the bond freed a reason, and "the
    /// peer is not advertising" is the genuine precondition that replaces
    /// it. Unlike the bond, this one is something the user can see and act
    /// on -- the other device is off, out of range, or has Bluetooth
    /// disabled.
    BluetoothPeerNotNearby,
    /// Bluetooth row, `Unconfigured`: preconditions are otherwise met, but
    /// automatic dial retries gave up after `ble::AUTO_RETRY_WINDOW` of no
    /// successful auth (real device evidence: a flaky link that connects,
    /// negotiates MTU, completes service discovery, then dies before the
    /// app-level Auth reply -- repeatedly, for minutes, with an indefinite
    /// "Still connecting..." the only visible symptom). Distinct from every
    /// other `Unconfigured` code: those describe a *precondition* that
    /// isn't met; this one means the precondition *was* met and dialling
    /// genuinely tried and failed. The row stays clickable in this state --
    /// see `ble::retry_bluetooth_dial` -- specifically to resume trying.
    BluetoothDialExhausted,
    /// `Configured`, amber: preconditions are met but no session is
    /// claimed on this channel yet -- a dial is presumably in flight (or
    /// about to be). Distinct from `AwaitingFirstAck`: that means a session
    /// *is* claimed and the ping/ack proof just hasn't completed its first
    /// round yet, which the frontend surfaces as "Connected -- waiting for
    /// the first ping/ack exchange." Reporting that same text here would be
    /// actively misleading for the common case of a presenced peer whose
    /// WebSocket port is unreachable -- no session has ever existed, let
    /// alone one about to prove itself.
    Connecting,
    /// `Configured`, amber: a session is claimed on this channel but the
    /// bidirectional ping/ack proof hasn't completed even once yet.
    AwaitingFirstAck,
    /// `Configured`, amber: the bidirectional ping/ack proof was complete
    /// at some point but has since lapsed -- `count` is the number of
    /// consecutive missed cycles on whichever side (own outbound or the
    /// peer's inbound) is currently behind. See `ChannelAckState`.
    PingMissed { count: u32 },
}

/// Unified per-row status shape (ADR-0003 revision): each channel
/// supplies its own condition logic for which state applies (see
/// `build_channel_statuses`), but the UI only ever has to render these
/// two cases, for either row. There is no separate "Live" case any more --
/// with both channels potentially connected and green at once, "which
/// one is carrying real traffic" is `ChannelStatus::primary`, orthogonal
/// to a row's own gray/amber/green state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RowState {
    /// Gray: local preconditions for this channel aren't met at all --
    /// nothing to prove yet, it simply can't carry a session right now.
    Unconfigured { code: ChannelStatusCode },
    /// Amber (`code: Some`) or green (`code: None`): preconditions are met
    /// and a session is claimed on this channel. Green requires the
    /// bidirectional ping/ack proof to be currently complete
    /// (`DeviceConnectionState::channel_reliable`) -- "continuously
    /// re-proven," not sticky, so a lapsed proof falls back to amber on its
    /// own without the channel having disconnected.
    Configured { code: Option<ChannelStatusCode> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub kind: ChannelKind,
    /// Whether this pair has this channel set up at all -- a `channels` row
    /// exists. `false` is the page's "you have not added this yet" state,
    /// which no boolean could express before rows were lazy.
    pub configured: bool,
    /// The switch. Always `false` when `configured` is.
    pub enabled: bool,
    /// The channel the person chose to carry this pair's traffic.
    ///
    /// A setting, not a live state (ADR-0007): it is `channels.is_primary`,
    /// so it survives a reconnect, governs the next one, and is shown
    /// whether or not this channel is connected at this moment. `false` on
    /// every row means they have not chosen, and selection falls back to the
    /// automatic network-first rule.
    pub primary: bool,
    /// Where this channel last reached the peer -- diagnostics only, and
    /// `None` until it has reached it once. Nothing dials it (ADR-0006).
    pub address: Option<String>,
    pub state: RowState,
}

/// Lightweight, in-memory-only per-channel liveness -- the same signal
/// `ChannelStatus`/`RowState` carries, minus everything that needs a DB
/// read or an OS-level check (the switch, `network_present`).
/// `device_connection_session_channel`'s live-poll sibling, polled far more
/// often than the full status read: green/amber is a continuously-reproven
/// ping/ack proof that can lapse or complete without anything the
/// network-presence-gated full poll would notice for a Bluetooth-only peer.
///
/// Carries no `primary`. The star is `channels.is_primary` -- a setting the
/// person made, which cannot change between two polls of a liveness signal,
/// and letting a live value overwrite it here is exactly how a persisted
/// choice would appear to move on its own.
///
/// `connected: false` means "no session on this channel" -- `code` is `None`
/// in that case too (nothing to say without the heavier check that knows
/// *why*); the frontend leaves `state` as last-known rather than inferring
/// `Unconfigured` from this alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelLiveness {
    pub kind: ChannelKind,
    pub connected: bool,
    pub code: Option<ChannelStatusCode>,
    /// `ble::is_bluetooth_dial_exhausted` -- always `false` for the network
    /// row. A P1 review finding: without this, the 5s live-poll timer
    /// (`refreshLiveConnectedState`, chosen specifically to avoid this
    /// struct's heavier DB-backed sibling's `bluetoothctl` subprocess cost
    /// on every tick) had no way to represent the exhausted state and
    /// unconditionally rewrote a disconnected row back to "connecting" --
    /// a Bluetooth-only peer never enters the network-presence-gated path
    /// that would otherwise trigger a fresh full `device_connection_
    /// channel_statuses` load to correct it, so the row stayed stuck
    /// showing "Still connecting..." forever, defeating this whole feature.
    pub dial_exhausted: bool,
}

pub fn select_channel_endpoint(
    peer_device_id: &str,
    network_endpoint: Option<ChannelEndpoint>,
    bluetooth: Option<BluetoothChannelMetadata>,
) -> Option<ChannelEndpoint> {
    if let Some(endpoint) = network_endpoint {
        return Some(endpoint);
    }

    let bluetooth = bluetooth?;
    if !bluetooth.enabled || !bluetooth.os_paired || bluetooth.address.trim().is_empty() {
        return None;
    }

    Some(ChannelEndpoint {
        peer_device_id: peer_device_id.to_string(),
        kind: ChannelKind::Bluetooth,
        address: bluetooth.address,
        ws_port: 0,
    })
}

/// Everything `build_channel_statuses` needs, one field per condition it
/// checks. A named-field struct instead of positional bools deliberately:
/// transposing two same-typed bools at a call site is exactly the class of
/// bug a struct's field names catch that positional args don't.
#[derive(Debug, Clone)]
pub struct ChannelStatusInputs {
    /// Whether a `channels` row exists for this pair and kind: the channel
    /// has been set up. A channel that was never set up is off for the same
    /// reason a switched-off one is, so the codes below do not distinguish
    /// them -- the page does, by offering to set it up.
    pub network_configured: bool,
    pub bluetooth_configured: bool,
    /// `channels.enabled` -- the per-pair switch for each channel.
    pub network_enabled: bool,
    pub bluetooth_enabled: bool,
    /// `channels.address` -- where the channel last reached the peer.
    pub network_address: Option<String>,
    pub bluetooth_address: Option<String>,
    /// Why this channel cannot reach the peer, or `None` if nothing is in
    /// the way — `ChannelService::why_not`.
    ///
    /// Answered by the channel itself rather than computed here: only the
    /// Network channel knows what presence it has heard, and only the
    /// Bluetooth one knows whether its radio is off, whether the peer was
    /// last seen advertising, and whether dialling has given up. The
    /// decision tables those answers come from are
    /// `network_unconfigured_code` and `bluetooth_unconfigured_code` below,
    /// kept pure so their ordering stays testable without a radio.
    pub network_unconfigured_code: Option<ChannelStatusCode>,
    pub bluetooth_unconfigured_code: Option<ChannelStatusCode>,
    /// Whether a session is currently claimed on this channel --
    /// `DeviceConnectionState::has_session_on`.
    pub network_connected: bool,
    pub bluetooth_connected: bool,
    /// `channels.is_primary` -- the channel the person chose, not whichever
    /// one happens to be carrying traffic. Both `false` means they have not
    /// chosen and selection is automatic.
    pub network_primary: bool,
    pub bluetooth_primary: bool,
    /// `DeviceConnectionState::channel_liveness_code` -- the amber
    /// reason, or `None` for green. Only consulted when `*_connected` is
    /// true.
    pub network_code: Option<ChannelStatusCode>,
    pub bluetooth_code: Option<ChannelStatusCode>,
}

pub fn build_channel_statuses(inputs: ChannelStatusInputs) -> Vec<ChannelStatus> {
    let ChannelStatusInputs {
        network_configured,
        bluetooth_configured,
        network_enabled,
        bluetooth_enabled,
        network_address,
        bluetooth_address,
        network_unconfigured_code,
        bluetooth_unconfigured_code,
        network_connected,
        bluetooth_connected,
        network_primary,
        bluetooth_primary,
        network_code,
        bluetooth_code,
    } = inputs;

    vec![
        ChannelStatus {
            kind: ChannelKind::Network,
            configured: network_configured,
            enabled: network_enabled,
            primary: network_primary,
            address: network_address,
            state: row_state(network_unconfigured_code, network_connected, network_code),
        },
        ChannelStatus {
            kind: ChannelKind::Bluetooth,
            configured: bluetooth_configured,
            enabled: bluetooth_enabled,
            primary: bluetooth_primary,
            address: bluetooth_address,
            state: row_state(bluetooth_unconfigured_code, bluetooth_connected, bluetooth_code),
        },
    ]
}

/// Shared shape for both rows: given whether (and why) a channel isn't
/// configured at all, and if it is, whether a session is claimed and what
/// amber code (if any) applies, decide which `RowState` case applies.
/// Transport-specific work stays in each channel's own "why unconfigured"
/// logic (`bluetooth_unconfigured_code`, inline for Network above) -- this
/// function only knows the shared shape.
fn row_state(
    unconfigured_code: Option<ChannelStatusCode>,
    connected: bool,
    code: Option<ChannelStatusCode>,
) -> RowState {
    if let Some(code) = unconfigured_code {
        return RowState::Unconfigured { code };
    }
    if !connected {
        return RowState::Configured {
            code: Some(ChannelStatusCode::Connecting),
        };
    }
    RowState::Configured { code }
}

/// ADR-0006 replaced the two arms that used to live here. A stored address
/// and a live OS bond were both preconditions; neither is any more, since a
/// peer is found by its advertisement and identified by the `Auth` frame.
///
/// In their place is one real precondition: whether the peer is advertising
/// at all. Ordered after `enabled` and before `dial_exhausted` on purpose --
/// a peer that was never in range has not "failed to connect after a minute
/// of trying", and saying so would send the user to retry a dial that has
/// nothing to dial.
/// Why the Network channel cannot reach a peer.
///
/// `enabled` is checked before presence deliberately: a channel the person
/// switched off must say so rather than blaming the peer's network, which
/// is a claim about the wrong machine and one they cannot act on.
pub fn network_unconfigured_code(enabled: bool, present: bool) -> Option<ChannelStatusCode> {
    if !enabled {
        return Some(ChannelStatusCode::NetworkDisabled);
    }
    if !present {
        return Some(ChannelStatusCode::NetworkUnavailable);
    }
    None
}

pub fn bluetooth_unconfigured_code(
    implemented: bool, enabled: bool, adapter_available: bool, peer_nearby: bool,
    dial_exhausted: bool,
) -> Option<ChannelStatusCode> {
    if !implemented {
        return Some(ChannelStatusCode::BluetoothNotSupported);
    }
    if !enabled {
        return Some(ChannelStatusCode::BluetoothDisabled);
    }
    if !adapter_available {
        return Some(ChannelStatusCode::BluetoothAdapterOff);
    }
    if !peer_nearby {
        return Some(ChannelStatusCode::BluetoothPeerNotNearby);
    }
    if dial_exhausted {
        return Some(ChannelStatusCode::BluetoothDialExhausted);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_endpoint(peer_device_id: &str) -> ChannelEndpoint {
        ChannelEndpoint {
            peer_device_id: peer_device_id.to_string(),
            kind: ChannelKind::Network,
            address: "192.168.1.10".to_string(),
            ws_port: 45455,
        }
    }

    fn bluetooth_metadata(peer_device_id: &str) -> BluetoothChannelMetadata {
        BluetoothChannelMetadata {
            peer_device_id: peer_device_id.to_string(),
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            enabled: true,
            os_paired: true,
        }
    }

    /// All inputs "healthy and ready" by default -- individual tests
    /// override just the field(s) they're exercising.
    fn ready_inputs() -> ChannelStatusInputs {
        ChannelStatusInputs {

            network_configured: true,
            bluetooth_configured: true,
            network_enabled: true,
            bluetooth_enabled: true,
            network_address: None,
            bluetooth_address: None,

            network_unconfigured_code: None,
            bluetooth_unconfigured_code: None,
            network_connected: false,
            bluetooth_connected: false,
            network_primary: false,
            bluetooth_primary: false,
            network_code: None,
            bluetooth_code: None,
        }
    }

    fn find(statuses: &[ChannelStatus], kind: ChannelKind) -> ChannelStatus {
        statuses
            .iter()
            .find(|status| status.kind == kind)
            .cloned()
            .unwrap_or_else(|| panic!("no {kind:?} status row"))
    }

    #[test]
    fn network_channel_is_preferred_when_both_are_available() {
        let selected = select_channel_endpoint(
            "peer-a",
            Some(network_endpoint("peer-a")),
            Some(bluetooth_metadata("peer-a")),
        )
        .expect("network should be selected");

        assert_eq!(selected.kind, ChannelKind::Network);
        assert_eq!(selected.address, "192.168.1.10");
    }

    #[test]
    fn bluetooth_channel_is_fallback_when_network_is_absent() {
        let selected =
            select_channel_endpoint("peer-a", None, Some(bluetooth_metadata("peer-a")))
                .expect("bluetooth should be selected");

        assert_eq!(selected.kind, ChannelKind::Bluetooth);
        assert_eq!(selected.address, "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn bluetooth_channel_requires_explicit_enablement_metadata_and_os_pairing() {
        for metadata in [
            BluetoothChannelMetadata {
                enabled: false,
                ..bluetooth_metadata("peer-a")
            },
            BluetoothChannelMetadata {
                address: "".to_string(),
                ..bluetooth_metadata("peer-a")
            },
            BluetoothChannelMetadata {
                os_paired: false,
                ..bluetooth_metadata("peer-a")
            },
        ] {
            assert_eq!(
                select_channel_endpoint("peer-a", None, Some(metadata)),
                None
            );
        }
    }

    #[test]
    fn a_channel_with_no_session_reports_unconfigured_or_connecting() {
        let not_present = build_channel_statuses(ChannelStatusInputs {
            network_unconfigured_code: Some(ChannelStatusCode::NetworkUnavailable),
            ..ready_inputs()
        });
        assert_eq!(
            find(&not_present, ChannelKind::Network).state,
            RowState::Unconfigured {
                code: ChannelStatusCode::NetworkUnavailable
            }
        );

        // Present but not yet connected: not unconfigured (preconditions
        // are met), but no session claimed yet either -- a P1 review
        // finding on this PR: this must not report `AwaitingFirstAck`
        // (which implies a session *is* claimed and only the ping/ack
        // proof is pending), or a peer whose WebSocket port is
        // permanently unreachable would misleadingly read as "connected."
        let presenced_only = build_channel_statuses(ready_inputs());
        assert_eq!(
            find(&presenced_only, ChannelKind::Network).state,
            RowState::Configured {
                code: Some(ChannelStatusCode::Connecting)
            }
        );
    }

    /// ADR-0003 revision's core new behavior: a claimed session isn't green
    /// on its own -- the bidirectional ping/ack proof (surfaced via
    /// `network_code`/`bluetooth_code`) decides amber vs green, independent
    /// of `primary`. A channel can be green and not primary (both
    /// connected and reliable; only one carries real traffic).
    #[test]
    fn green_requires_a_completed_ack_proof_independent_of_primary() {
        let statuses = build_channel_statuses(ChannelStatusInputs {
            network_connected: true,
            network_primary: true,
            network_code: None,
            bluetooth_connected: true,
            bluetooth_primary: false,
            bluetooth_code: None,
            ..ready_inputs()
        });
        let network = find(&statuses, ChannelKind::Network);
        let bluetooth = find(&statuses, ChannelKind::Bluetooth);
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
    fn a_connected_channel_awaiting_or_missing_ack_reports_amber() {
        let awaiting = build_channel_statuses(ChannelStatusInputs {
            network_connected: true,
            network_code: Some(ChannelStatusCode::AwaitingFirstAck),
            ..ready_inputs()
        });
        assert_eq!(
            find(&awaiting, ChannelKind::Network).state,
            RowState::Configured {
                code: Some(ChannelStatusCode::AwaitingFirstAck)
            }
        );

        let lapsed = build_channel_statuses(ChannelStatusInputs {
            network_connected: true,
            network_code: Some(ChannelStatusCode::PingMissed { count: 2 }),
            ..ready_inputs()
        });
        assert_eq!(
            find(&lapsed, ChannelKind::Network).state,
            RowState::Configured {
                code: Some(ChannelStatusCode::PingMissed { count: 2 })
            }
        );
    }

    /// The Bluetooth reasons, in the order they are checked.
    ///
    /// The ordering is load-bearing rather than cosmetic, and every case here
    /// is a claim about a *different machine*: "switched off" is about the
    /// pair, "this computer's radio is off" is about this device, "isn't
    /// nearby" is about the peer. Getting the order wrong does not produce a
    /// slightly worse message, it produces a confident statement about the
    /// wrong device that the person cannot act on.
    ///
    /// Tested against the decision table directly rather than through a row,
    /// and on every platform: `implemented` is a parameter now that the
    /// Bluetooth service reads it from its radio, so the no-adapter case no
    /// longer needs a build that has no adapter.
    #[test]
    fn bluetooth_reasons_are_checked_in_the_order_that_names_the_right_device() {
        let cases = [
            (
                "no adapter on this platform at all outranks everything",
                (false, false, false, false, true),
                Some(ChannelStatusCode::BluetoothNotSupported),
            ),
            (
                "a channel the person switched off has no business \
                 complaining about hardware",
                (true, false, false, false, true),
                Some(ChannelStatusCode::BluetoothDisabled),
            ),
            (
                "with our own radio off nothing has scanned, so 'isn't \
                 nearby' would assert what we never looked for",
                (true, true, false, false, true),
                Some(ChannelStatusCode::BluetoothAdapterOff),
            ),
            (
                "a peer that was never in range has not failed to connect \
                 after a minute of trying",
                (true, true, true, false, true),
                Some(ChannelStatusCode::BluetoothPeerNotNearby),
            ),
            (
                "exhaustion means anything only once dialling was actually \
                 attempted, so it is checked last",
                (true, true, true, true, true),
                Some(ChannelStatusCode::BluetoothDialExhausted),
            ),
            (
                "nothing in the way",
                (true, true, true, true, false),
                None,
            ),
        ];

        for (why, (implemented, enabled, adapter, nearby, exhausted), expected) in cases {
            assert_eq!(
                bluetooth_unconfigured_code(implemented, enabled, adapter, nearby, exhausted),
                expected,
                "{why}"
            );
        }
    }

    /// The Network switch outranks presence for the same reason the Bluetooth
    /// one does: a channel the person turned off must not blame the peer's
    /// network for being unreachable. Asserted with the peer *present* as
    /// well, because that is when the switch matters most and when getting
    /// this wrong would make it look like it did nothing.
    #[test]
    fn the_network_switch_outranks_presence() {
        assert_eq!(
            network_unconfigured_code(false, false),
            Some(ChannelStatusCode::NetworkDisabled)
        );
        assert_eq!(
            network_unconfigured_code(false, true),
            Some(ChannelStatusCode::NetworkDisabled),
        );
        assert_eq!(
            network_unconfigured_code(true, false),
            Some(ChannelStatusCode::NetworkUnavailable)
        );
        assert_eq!(network_unconfigured_code(true, true), None);
    }

    /// A reason from the channel becomes a gray row; no reason and no session
    /// becomes "Connecting…". This is all `build_channel_statuses` decides
    /// about reachability now -- the reasons themselves come from whichever
    /// channel knows, and the two tests above cover those.
    #[test]
    fn a_reason_makes_the_row_gray_and_its_absence_makes_it_connecting() {
        let statuses = build_channel_statuses(ChannelStatusInputs {
            bluetooth_unconfigured_code: Some(ChannelStatusCode::BluetoothPeerNotNearby),
            ..ready_inputs()
        });

        assert_eq!(
            find(&statuses, ChannelKind::Bluetooth).state,
            RowState::Unconfigured {
                code: ChannelStatusCode::BluetoothPeerNotNearby
            }
        );
        assert_eq!(
            find(&statuses, ChannelKind::Network).state,
            RowState::Configured {
                code: Some(ChannelStatusCode::Connecting)
            }
        );
    }
}
