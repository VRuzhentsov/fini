//! What a channel row on the Device page shows (ADR-0008 D19).
//!
//! The backend derives one presentation per row from the stored channel
//! state (D15: `None`, `Off`, `On`) and two runtime facts -- whether the
//! peer is present on the channel (D9), and whether the channel reports a
//! problem on this device (D6). The frontend draws it and derives nothing.
//!
//! | Colour | Meaning | Stored state |
//! |---|---|---|
//! | green | peer seen within the channel timeout | `On` |
//! | grey | peer not seen | `On` |
//! | orange | problem on this device's channel | `On` |
//! | off | switched off | `Off` |
//! | none | not added | `None` |

#[cfg(any(feature = "ui-plane", test))]
use serde::{Deserialize, Serialize};

pub use crate::services::communication::channel::ChannelKind;

/// A channel's stored state (ADR-0008 D15). `None` is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg(any(feature = "ui-plane", test))]
pub enum ChannelState {
    None,
    Off,
    On,
}

/// The row's colour (ADR-0008 D19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg(any(feature = "ui-plane", test))]
pub enum ChannelColor {
    Green,
    Grey,
    Orange,
    Off,
    None,
}

/// A problem on this device's channel -- what the ⓘ popup on an orange row
/// explains. Each channel reports its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg(any(feature = "ui-plane", test))]
pub enum ChannelProblem {
    /// This platform has no Bluetooth support at all.
    BluetoothNotSupported,
    /// Bluetooth is off, missing, or refusing to work on this device.
    BluetoothUnavailable,
    /// This device cannot announce itself on the local network.
    NetworkUnavailable,
}

/// One row on the Device page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(any(feature = "ui-plane", test))]
pub struct ChannelStatus {
    pub kind: ChannelKind,
    pub state: ChannelState,
    pub color: ChannelColor,
    /// Set only on an orange row.
    pub problem: Option<ChannelProblem>,
}

/// The whole of D19's table as one function.
///
/// A problem outranks presence: a peer "seen" through a channel this
/// device cannot use is not a peer data can reach, and the person can only
/// act on the problem.
#[cfg(any(feature = "ui-plane", test))]
pub fn channel_status(
    kind: ChannelKind,
    state: ChannelState,
    present: bool,
    problem: Option<ChannelProblem>,
) -> ChannelStatus {
    let (color, problem) = match state {
        ChannelState::None => (ChannelColor::None, None),
        ChannelState::Off => (ChannelColor::Off, None),
        ChannelState::On => match problem {
            Some(problem) => (ChannelColor::Orange, Some(problem)),
            None if present => (ChannelColor::Green, None),
            None => (ChannelColor::Grey, None),
        },
    };
    ChannelStatus {
        kind,
        state,
        color,
        problem,
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
        for (state, present, reported, color, shown) in cases {
            let row = channel_status(ChannelKind::Bluetooth, state, present, reported);
            assert_eq!(
                (row.color, row.problem),
                (color, shown),
                "{state:?}, present={present}, problem={reported:?}"
            );
        }
    }
}
