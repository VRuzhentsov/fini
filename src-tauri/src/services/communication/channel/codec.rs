//! `PeerFrame <-> bytes`, via the versioned envelope and the active
//! `SecureChannel` (pass-through today). Shared by every adapter so the
//! wire format is identical regardless of which `DataLink` carries it.

use crate::services::communication::sync::types::PeerFrame;
use crate::services::communication::channel::envelope::{EncScheme, FrameEnvelope, ENVELOPE_VERSION};
use crate::services::communication::channel::encryption::{PlaintextChannel, SecureChannel};

fn channel() -> impl SecureChannel {
    PlaintextChannel
}

/// `PeerFrame -> ciphertext-in-envelope -> bytes`, ready to hand to `DataLink::send`.
pub fn encode_frame(frame: &PeerFrame) -> Result<Vec<u8>, String> {
    let plain = serde_json::to_vec(frame).map_err(|err| format!("encode PeerFrame: {err}"))?;
    let channel = channel();
    let payload = channel.encrypt(plain)?;
    let envelope = FrameEnvelope::new(channel.scheme(), payload);
    serde_json::to_vec(&envelope).map_err(|err| format!("encode envelope: {err}"))
}

/// Bytes from `DataLink::recv` -> envelope -> plaintext -> `PeerFrame`.
pub fn decode_frame(bytes: &[u8]) -> Result<PeerFrame, String> {
    let envelope: FrameEnvelope =
        serde_json::from_slice(bytes).map_err(|err| format!("decode envelope: {err}"))?;
    if envelope.v != ENVELOPE_VERSION {
        return Err(format!("unsupported envelope version {}", envelope.v));
    }
    if envelope.enc != EncScheme::None {
        return Err(format!(
            "unsupported encryption scheme {:?} (no SecureChannel impl yet)",
            envelope.enc
        ));
    }
    let channel = channel();
    let plain = channel.decrypt(envelope.payload)?;
    serde_json::from_slice(&plain).map_err(|err| format!("decode PeerFrame: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_peer_frame() {
        let frame = PeerFrame::AuthOk { protocol_version: 1 };
        let bytes = encode_frame(&frame).expect("encode");
        let decoded = decode_frame(&bytes).expect("decode");
        assert!(matches!(decoded, PeerFrame::AuthOk { protocol_version: 1 }));
    }

    #[test]
    fn envelope_carries_version_and_none_scheme() {
        let bytes =
            encode_frame(&PeerFrame::AuthOk { protocol_version: 1 }).expect("encode");
        let envelope: FrameEnvelope = serde_json::from_slice(&bytes).expect("parse envelope");
        assert_eq!(envelope.v, ENVELOPE_VERSION);
        assert_eq!(envelope.enc, EncScheme::None);
    }

    /// Regression test: a frame `type` this build doesn't recognize (e.g.
    /// one added by a newer peer) must decode into `PeerFrame::Unknown`
    /// rather than fail outright -- otherwise `run_session`'s `let
    /// Some(Ok(frame)) = inbound else { break }` would treat any single
    /// unrecognized frame as a fatal decode error and silently end the
    /// whole authenticated sync session. Mixed-version paired devices are
    /// the normal case during a rollout, not an edge case.
    #[test]
    fn unrecognized_frame_type_decodes_to_unknown_instead_of_failing() {
        let inner = serde_json::json!({
            "type": "some_frame_kind_this_build_has_never_heard_of",
            "extra_field": 123,
        });
        let plain = serde_json::to_vec(&inner).unwrap();
        let envelope = FrameEnvelope::new(EncScheme::None, plain);
        let bytes = serde_json::to_vec(&envelope).unwrap();

        let decoded = decode_frame(&bytes).expect("must decode, not error");
        assert!(matches!(decoded, PeerFrame::Unknown));
    }

    #[test]
    fn rejects_unsupported_envelope_version() {
        let envelope = FrameEnvelope {
            v: 99,
            enc: EncScheme::None,
            payload: serde_json::to_vec(&PeerFrame::AuthOk { protocol_version: 1 }).unwrap(),
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let err = decode_frame(&bytes).expect_err("should reject");
        assert!(err.contains("unsupported envelope version"));
    }

}
