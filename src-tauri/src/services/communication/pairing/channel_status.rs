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

/// What a channel row *is*, independent of which channel it is.
///
/// The category, and the only thing that decides the colour of the dot —
/// deliberately not named as a colour, because the name has to survive a
/// redesign that repaints them.
///
/// This used to be derived in the frontend, by switching on Bluetooth
/// codes: `bluetooth_adapter_off` and `bluetooth_not_supported` meant
/// "waiting", everything else meant "down". So the one piece of code that
/// is supposed to be channel-agnostic was the piece that had to know one
/// channel's failure modes, and a third channel would have had to teach it
/// more. A channel categorises its own reasons now; only the category
/// crosses the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelRowState {
    /// The switch is off. Nothing is happening because the person said so.
    Off,
    /// On, and what is missing is on *this* machine — so it is not a
    /// failure, and the channel starts by itself once the condition clears.
    Waiting,
    /// On, this machine is fine, and the other device is out of reach.
    Down,
    /// Dialling, or dialled and not yet proven.
    Connecting,
    /// Proven live, and the proof has since lapsed.
    Fading,
    /// Proven live right now.
    Connected,
}

/// Reasons that read the same on every channel.
///
/// `Disabled` replaces what used to be `NetworkDisabled` and
/// `BluetoothDisabled`: two codes for one sentence with the channel's name
/// substituted into it, which is a thing the wording layer can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatusCode {
    /// Switched off for this pair.
    Disabled,
    /// A session is being established.
    Connecting,
    /// Connected, and the first ping/ack proof has not completed yet.
    AwaitingFirstAck,
    /// The proof was complete and has lapsed.
    NotAnswering,
}

/// Why the Network channel cannot reach this peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatusCode {
    /// No discovery presence for this peer.
    PeerNotOnNetwork,
}

/// Why the Bluetooth channel cannot reach this peer.
///
/// The ordering these are produced in is load-bearing rather than
/// cosmetic — see `bluetooth_unconfigured_code`. Each one is a claim about
/// a *different machine*, and the wrong order states something confident
/// about the wrong device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BluetoothStatusCode {
    /// No adapter on this platform at all: no build of this app on this OS
    /// could use it.
    NotSupported,
    /// The local radio refused the last time this process tried to use it —
    /// switched off at the OS level, or present and declining to scan.
    AdapterOff,
    /// Enabled, with no address learned yet.
    NoAddress,
    /// Not heard advertising recently: off, out of range, or its own
    /// Bluetooth is disabled.
    PeerNotNearby,
    /// Dialling tried and gave up. The one reason a person can act on by
    /// asking for another try.
    DialExhausted,
}

/// One reason, whichever kind it is.
///
/// The type split is what the channels themselves work in; this is how a
/// reason travels once it has been decided. Each variant knows two things:
/// the category it belongs to, and the stable key the wording is looked up
/// by — so adding a reason cannot forget to say which colour it implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelReason {
    Any(ChannelStatusCode),
    Network(NetworkStatusCode),
    Bluetooth(BluetoothStatusCode),
}

impl ChannelReason {
    /// The category this reason puts the row in.
    pub fn category(self) -> ChannelRowState {
        match self {
            ChannelReason::Any(ChannelStatusCode::Disabled) => ChannelRowState::Off,
            ChannelReason::Any(ChannelStatusCode::Connecting)
            | ChannelReason::Any(ChannelStatusCode::AwaitingFirstAck) => ChannelRowState::Connecting,
            ChannelReason::Any(ChannelStatusCode::NotAnswering) => ChannelRowState::Fading,
            ChannelReason::Network(NetworkStatusCode::PeerNotOnNetwork) => ChannelRowState::Down,
            // The two that are about this machine rather than the peer. The
            // switch is on and the missing piece is here, so the person is
            // waiting rather than looking at a failure.
            ChannelReason::Bluetooth(BluetoothStatusCode::NotSupported)
            | ChannelReason::Bluetooth(BluetoothStatusCode::AdapterOff) => ChannelRowState::Waiting,
            ChannelReason::Bluetooth(_) => ChannelRowState::Down,
        }
    }

    /// The stable key the wording is keyed by. Chosen so a locale table can
    /// translate without the backend changing.
    pub fn code(self) -> &'static str {
        match self {
            ChannelReason::Any(ChannelStatusCode::Disabled) => "disabled",
            ChannelReason::Any(ChannelStatusCode::Connecting) => "connecting",
            ChannelReason::Any(ChannelStatusCode::AwaitingFirstAck) => "awaiting_first_ack",
            ChannelReason::Any(ChannelStatusCode::NotAnswering) => "not_answering",
            ChannelReason::Network(NetworkStatusCode::PeerNotOnNetwork) => "peer_not_on_network",
            ChannelReason::Bluetooth(BluetoothStatusCode::NotSupported) => "bluetooth_not_supported",
            ChannelReason::Bluetooth(BluetoothStatusCode::AdapterOff) => "bluetooth_adapter_off",
            ChannelReason::Bluetooth(BluetoothStatusCode::NoAddress) => "bluetooth_no_address",
            ChannelReason::Bluetooth(BluetoothStatusCode::PeerNotNearby) => "bluetooth_peer_not_nearby",
            ChannelReason::Bluetooth(BluetoothStatusCode::DialExhausted) => "bluetooth_dial_exhausted",
        }
    }

    /// Whether this is the one reason a person can act on by asking for
    /// another try. Read by the row instead of matching a Bluetooth code,
    /// which is how the retry button used to decide.
    pub fn retryable(self) -> bool {
        matches!(self, ChannelReason::Bluetooth(BluetoothStatusCode::DialExhausted))
    }
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
    /// The category: what this row is, and the only thing the colour is
    /// decided from.
    pub status: ChannelRowState,
    /// The stable key for the sentence behind the information button, or
    /// `None` when the row has nothing to explain. Never shown as-is.
    ///
    /// Two fields rather than one code carrying both jobs: the category is
    /// the same question on every channel, the sentence is not, and mixing
    /// them is what forced the frontend to learn Bluetooth's failure modes
    /// in order to pick a colour.
    pub reason: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelLiveness {
    pub kind: ChannelKind,
    pub connected: bool,
    /// The stable key for the sentence, matching `ChannelStatus::reason`.
    /// A key rather than a typed code because this shape is a DTO for a
    /// poll the frontend reads directly; the typed enums are what the
    /// channels decide in.
    pub reason: Option<String>,
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
    pub network_unconfigured_code: Option<ChannelReason>,
    pub bluetooth_unconfigured_code: Option<ChannelReason>,
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
    pub network_code: Option<ChannelReason>,
    pub bluetooth_code: Option<ChannelReason>,
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
            status: row_status(network_unconfigured_code, network_connected, network_code).0,
            reason: row_status(network_unconfigured_code, network_connected, network_code).1,
        },
        ChannelStatus {
            kind: ChannelKind::Bluetooth,
            configured: bluetooth_configured,
            enabled: bluetooth_enabled,
            primary: bluetooth_primary,
            address: bluetooth_address,
            status: row_status(bluetooth_unconfigured_code, bluetooth_connected, bluetooth_code).0,
            reason: row_status(bluetooth_unconfigured_code, bluetooth_connected, bluetooth_code).1,
        },
    ]
}

/// Shared shape for both rows: given whether (and why) a channel cannot
/// carry a session, and if it can, whether one is claimed and proven,
/// decide the category and the sentence.
///
/// It no longer decides *which* category a reason implies — the reason
/// does, via `ChannelReason::category`. This only knows the shape every
/// channel has in common.
fn row_status(
    unconfigured: Option<ChannelReason>,
    connected: bool,
    code: Option<ChannelReason>,
) -> (ChannelRowState, Option<String>) {
    if let Some(reason) = unconfigured {
        return (reason.category(), Some(reason.code().to_string()));
    }
    if !connected {
        let reason = ChannelReason::Any(ChannelStatusCode::Connecting);
        return (reason.category(), Some(reason.code().to_string()));
    }
    match code {
        Some(reason) => (reason.category(), Some(reason.code().to_string())),
        None => (ChannelRowState::Connected, None),
    }
}

pub fn network_unconfigured_code(enabled: bool, present: bool) -> Option<ChannelReason> {
    if !enabled {
        return Some(ChannelReason::Any(ChannelStatusCode::Disabled));
    }
    if !present {
        return Some(ChannelReason::Network(NetworkStatusCode::PeerNotOnNetwork));
    }
    None
}

pub fn bluetooth_unconfigured_code(
    implemented: bool, enabled: bool, adapter_available: bool, peer_nearby: bool,
    dial_exhausted: bool,
) -> Option<ChannelReason> {
    if !implemented {
        return Some(ChannelReason::Bluetooth(BluetoothStatusCode::NotSupported));
    }
    if !enabled {
        return Some(ChannelReason::Any(ChannelStatusCode::Disabled));
    }
    if !adapter_available {
        return Some(ChannelReason::Bluetooth(BluetoothStatusCode::AdapterOff));
    }
    if !peer_nearby {
        return Some(ChannelReason::Bluetooth(BluetoothStatusCode::PeerNotNearby));
    }
    if dial_exhausted {
        return Some(ChannelReason::Bluetooth(BluetoothStatusCode::DialExhausted));
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
            network_unconfigured_code: Some(ChannelReason::Network(NetworkStatusCode::PeerNotOnNetwork)),
            ..ready_inputs()
        });
        let row = find(&not_present, ChannelKind::Network);
        assert_eq!(row.status, ChannelRowState::Down);
        assert_eq!(row.reason.as_deref(), Some("peer_not_on_network"));

        // Present but not yet connected: not unconfigured (preconditions
        // are met), but no session claimed yet either -- a P1 review
        // finding on this PR: this must not report `AwaitingFirstAck`
        // (which implies a session *is* claimed and only the ping/ack
        // proof is pending), or a peer whose WebSocket port is
        // permanently unreachable would misleadingly read as "connected."
        let presenced_only = build_channel_statuses(ready_inputs());
        let row = find(&presenced_only, ChannelKind::Network);
        assert_eq!(row.status, ChannelRowState::Connecting);
        assert_eq!(row.reason.as_deref(), Some("connecting"));
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
        assert_eq!(network.status, ChannelRowState::Connected);
        assert_eq!(network.reason, None);
        assert!(network.primary);
        assert_eq!(
            bluetooth.status,
            ChannelRowState::Connected,
            "bluetooth can be green while not primary"
        );
        assert!(!bluetooth.primary);
    }

    #[test]
    fn a_connected_channel_awaiting_or_missing_ack_reports_amber() {
        let awaiting = build_channel_statuses(ChannelStatusInputs {
            network_connected: true,
            network_code: Some(ChannelReason::Any(ChannelStatusCode::AwaitingFirstAck)),
            ..ready_inputs()
        });
        let row = find(&awaiting, ChannelKind::Network);
        assert_eq!(row.status, ChannelRowState::Connecting);
        assert_eq!(row.reason.as_deref(), Some("awaiting_first_ack"));

        let lapsed = build_channel_statuses(ChannelStatusInputs {
            network_connected: true,
            network_code: Some(ChannelReason::Any(ChannelStatusCode::NotAnswering)),
            ..ready_inputs()
        });
        let row = find(&lapsed, ChannelKind::Network);
        assert_eq!(row.status, ChannelRowState::Fading);
        assert_eq!(row.reason.as_deref(), Some("not_answering"));
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
                Some(ChannelReason::Bluetooth(BluetoothStatusCode::NotSupported)),
            ),
            (
                "a channel the person switched off has no business \
                 complaining about hardware",
                (true, false, false, false, true),
                Some(ChannelReason::Any(ChannelStatusCode::Disabled)),
            ),
            (
                "with our own radio off nothing has scanned, so 'isn't \
                 nearby' would assert what we never looked for",
                (true, true, false, false, true),
                Some(ChannelReason::Bluetooth(BluetoothStatusCode::AdapterOff)),
            ),
            (
                "a peer that was never in range has not failed to connect \
                 after a minute of trying",
                (true, true, true, false, true),
                Some(ChannelReason::Bluetooth(BluetoothStatusCode::PeerNotNearby)),
            ),
            (
                "exhaustion means anything only once dialling was actually \
                 attempted, so it is checked last",
                (true, true, true, true, true),
                Some(ChannelReason::Bluetooth(BluetoothStatusCode::DialExhausted)),
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
            Some(ChannelReason::Any(ChannelStatusCode::Disabled))
        );
        assert_eq!(
            network_unconfigured_code(false, true),
            Some(ChannelReason::Any(ChannelStatusCode::Disabled)),
        );
        assert_eq!(
            network_unconfigured_code(true, false),
            Some(ChannelReason::Network(NetworkStatusCode::PeerNotOnNetwork))
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
            bluetooth_unconfigured_code: Some(ChannelReason::Bluetooth(BluetoothStatusCode::PeerNotNearby)),
            ..ready_inputs()
        });

        let bluetooth = find(&statuses, ChannelKind::Bluetooth);
        assert_eq!(bluetooth.status, ChannelRowState::Down);
        assert_eq!(bluetooth.reason.as_deref(), Some("bluetooth_peer_not_nearby"));

        let network = find(&statuses, ChannelKind::Network);
        assert_eq!(network.status, ChannelRowState::Connecting);
        assert_eq!(network.reason.as_deref(), Some("connecting"));
    }
}
