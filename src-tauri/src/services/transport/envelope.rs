//! Versioned wire envelope wrapping every encoded `PeerFrame`.
//!
//! The envelope exists so that turning on real encryption later (see
//! `secure_channel`) is an additive wire change, not a breaking one: the
//! version/scheme are already on every frame a shipped device has ever sent.

use serde::{Deserialize, Serialize};

pub const ENVELOPE_VERSION: u8 = 1;

/// Which `SecureChannel` produced `payload`. `None` today (pass-through);
/// reserved variants document the intended future scheme without
/// implementing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncScheme {
    /// No encryption; `payload` is the plain encoded `PeerFrame`.
    None,
    /// Reserved: Signal-style Double Ratchet, keyed by X3DH at pairing time.
    /// No implementation exists yet.
    SignalDoubleRatchet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameEnvelope {
    pub v: u8,
    pub enc: EncScheme,
    #[serde(with = "base64_payload")]
    pub payload: Vec<u8>,
}

/// `serde_json`'s default `Vec<u8>` representation is a JSON array of
/// numbers -- roughly 4 bytes on the wire per plaintext byte (digits plus a
/// comma), versus ~1.33x for base64. Harmless bandwidth waste over
/// tcp_ws/Sim; not over BLE, where it directly multiplies fragment count on
/// an already-small GATT budget. Found via the actors-ble e2e lane: a
/// ~400-byte `PeerFrame::SyncEvent` was landing on the wire at ~2850 bytes,
/// pushing one small quest edit to ~238 fragments and 20+ real seconds to
/// transmit on the peripheral role's 12-byte fragment budget -- long enough
/// to starve other traffic on the same session and read as an outright
/// delivery failure.
mod base64_payload {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(&encoded).map_err(serde::de::Error::custom)
    }
}

impl FrameEnvelope {
    pub fn new(enc: EncScheme, payload: Vec<u8>) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            enc,
            payload,
        }
    }
}
