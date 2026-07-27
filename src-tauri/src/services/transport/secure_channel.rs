//! The encryption seam. Crypto itself is out of scope for this PR; this
//! module is the room left for it: a `SecureChannel` sits between the
//! `PeerFrame` codec and the `Link`, and today does nothing
//! (`PlaintextChannel`). Enabling Signal-style E2E encryption later means
//! adding a new `SecureChannel` impl plus a real `KeyMaterial` payload
//! exchanged during pairing — not a wire-format break, because the
//! versioned envelope and this reserved field already exist.

use serde::{Deserialize, Serialize};

use crate::services::transport::envelope::EncScheme;

/// Reserved slot on the pair-complete handshake for future X3DH key
/// agreement material. Unused today; always `None` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMaterial {
    pub scheme: EncScheme,
    pub identity_key: String,
    pub signed_prekey: String,
}

pub trait SecureChannel: Send + Sync {
    fn scheme(&self) -> EncScheme;
    fn encrypt(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, String>;
    fn decrypt(&self, ciphertext: Vec<u8>) -> Result<Vec<u8>, String>;
}

/// Identity pass-through. The only `SecureChannel` implemented in this PR.
pub struct PlaintextChannel;

impl SecureChannel for PlaintextChannel {
    fn scheme(&self) -> EncScheme {
        EncScheme::None
    }

    fn encrypt(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, String> {
        Ok(plaintext)
    }

    fn decrypt(&self, ciphertext: Vec<u8>) -> Result<Vec<u8>, String> {
        Ok(ciphertext)
    }
}
