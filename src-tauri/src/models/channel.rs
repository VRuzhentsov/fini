use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::channels;

/// Which channel it is: one variant for each channel, and one session slot
/// for each variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// iroh over IP, found by iroh's mDNS lookup.
    #[default]
    Network,
    /// iroh over GATT, via `ble-gatt`.
    Bluetooth,
}

impl ChannelKind {
    /// How this kind is stored: `channels.channel_kind`, seeded into
    /// `channel_kinds.code`. The same strings serde produces, so the wire
    /// form and the stored form are the same.
    pub fn code(self) -> &'static str {
        match self {
            ChannelKind::Network => "network",
            ChannelKind::Bluetooth => "bluetooth",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "network" => Some(ChannelKind::Network),
            "bluetooth" => Some(ChannelKind::Bluetooth),
            _ => None,
        }
    }
}

/// One configured channel for one pair. The row existing is what "configured"
/// means; see `migrations/00000000000023_channels/up.sql`.
#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = channels)]
#[diesel(primary_key(device_id, channel_kind))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Channel {
    pub device_id: String,
    /// `ChannelKind`'s serde form -- `"network"` or `"bluetooth"`, the same
    /// strings seeded into `channel_kinds`.
    pub channel_kind: String,
    pub enabled: bool,
    /// The person's choice of this channel to carry the pair's traffic
    /// (ADR-0007). At most one per pair.
    pub is_primary: bool,
    /// The link-layer address this channel last reached the peer at, kept
    /// for diagnostics only -- ADR-0006 dials nothing by address.
    pub address: Option<String>,
    pub configured_at: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = channels)]
pub struct NewChannel {
    pub device_id: String,
    pub channel_kind: String,
    pub enabled: bool,
    pub is_primary: bool,
    pub address: Option<String>,
    pub configured_at: String,
}

/// A channel's stored state (ADR-0008 D15). `None` is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    None,
    Off,
    On,
}

/// The colour of a channel row (ADR-0008 D19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelColor {
    Green,
    Grey,
    Orange,
    Off,
    None,
}

/// A problem on this device's channel: what the ⓘ popup on an orange row
/// tells. Each channel reports its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProblem {
    /// This platform has no Bluetooth support at all.
    BluetoothNotSupported,
    /// Bluetooth is off, missing, or does not work on this device.
    BluetoothUnavailable,
    /// This device cannot announce itself on the local network.
    NetworkUnavailable,
    /// This pair has no pinned key, so no channel of it can connect. Pairs
    /// made before ADR-0009 have none; they are paired again.
    PairKeyMissing,
}

/// One channel row on the Device page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub kind: ChannelKind,
    pub state: ChannelState,
    pub color: ChannelColor,
    /// Set only on an orange row.
    pub problem: Option<ChannelProblem>,
    /// The person's chosen channel for this pair's traffic (ADR-0007).
    pub primary: bool,
}

impl ChannelStatus {
    /// The row for a channel (ADR-0008 D19). A problem outranks a live
    /// connection: the person can only act on the problem.
    pub fn new(kind: ChannelKind, state: ChannelState, green: bool, problem: Option<ChannelProblem>, primary: bool) -> Self {
        let (color, problem) = match state {
            ChannelState::None => (ChannelColor::None, None),
            ChannelState::Off => (ChannelColor::Off, None),
            ChannelState::On => match problem {
                Some(problem) => (ChannelColor::Orange, Some(problem)),
                None if green => (ChannelColor::Green, None),
                None => (ChannelColor::Grey, None),
            },
        };
        Self { kind, state, color, problem, primary }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_state_maps_to_exactly_one_row() {
        let problem = Some(ChannelProblem::BluetoothUnavailable);
        let cases = [
            (ChannelState::None, true, problem, ChannelColor::None, None),
            (ChannelState::Off, true, problem, ChannelColor::Off, None),
            (ChannelState::On, true, None, ChannelColor::Green, None),
            (ChannelState::On, false, None, ChannelColor::Grey, None),
            (ChannelState::On, true, problem, ChannelColor::Orange, problem),
            (ChannelState::On, false, problem, ChannelColor::Orange, problem),
        ];
        for (state, green, reported, color, shown) in cases {
            let row = ChannelStatus::new(ChannelKind::Bluetooth, state, green, reported, false);
            assert_eq!((row.color, row.problem), (color, shown), "{state:?}, green={green}, problem={reported:?}");
        }
    }
}
