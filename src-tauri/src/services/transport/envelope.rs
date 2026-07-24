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
    pub payload: Vec<u8>,
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
