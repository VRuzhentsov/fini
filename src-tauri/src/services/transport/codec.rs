//! `PeerFrame <-> bytes`, via the versioned envelope and the active
//! `SecureChannel` (pass-through today). Shared by every adapter so the
//! wire format is identical regardless of which `Link` carries it.

use crate::services::space_sync::types::PeerFrame;
use crate::services::transport::envelope::{EncScheme, FrameEnvelope, ENVELOPE_VERSION};
use crate::services::transport::secure_channel::{PlaintextChannel, SecureChannel};

fn channel() -> impl SecureChannel {
    PlaintextChannel
}

/// `PeerFrame -> ciphertext-in-envelope -> bytes`, ready to hand to `Link::send`.
pub fn encode_frame(frame: &PeerFrame) -> Result<Vec<u8>, String> {
    let plain = serde_json::to_vec(frame).map_err(|err| format!("encode PeerFrame: {err}"))?;
    let channel = channel();
    let payload = channel.encrypt(plain)?;
    let envelope = FrameEnvelope::new(channel.scheme(), payload);
    serde_json::to_vec(&envelope).map_err(|err| format!("encode envelope: {err}"))
}

/// Bytes from `Link::recv` -> envelope -> plaintext -> `PeerFrame`.
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

/// Length-delimited byte framing (4-byte big-endian length prefix) for
/// adapters that carry raw byte streams without their own message
/// boundaries (e.g. `sim`, and the future real Bluetooth adapter).
pub mod length_delimited {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

    pub async fn write<W: tokio::io::AsyncWrite + Unpin>(
        writer: &mut W,
        payload: &[u8],
    ) -> Result<(), String> {
        let len = u32::try_from(payload.len()).map_err(|_| "frame too large".to_string())?;
        writer
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|err| format!("write frame length: {err}"))?;
        writer
            .write_all(payload)
            .await
            .map_err(|err| format!("write frame payload: {err}"))?;
        writer
            .flush()
            .await
            .map_err(|err| format!("flush frame: {err}"))
    }

    /// `Ok(None)` means clean EOF (peer closed the connection).
    pub async fn read<R: tokio::io::AsyncRead + Unpin>(
        reader: &mut R,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut len_buf = [0_u8; 4];
        match reader.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(format!("read frame length: {err}")),
        }
        let len = u32::from_be_bytes(len_buf);
        if len > MAX_FRAME_LEN {
            return Err(format!("frame length {len} exceeds max {MAX_FRAME_LEN}"));
        }
        let mut payload = vec![0_u8; len as usize];
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|err| format!("read frame payload: {err}"))?;
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_peer_frame() {
        let frame = PeerFrame::AuthOk;
        let bytes = encode_frame(&frame).expect("encode");
        let decoded = decode_frame(&bytes).expect("decode");
        assert!(matches!(decoded, PeerFrame::AuthOk));
    }

    #[test]
    fn envelope_carries_version_and_none_scheme() {
        let bytes = encode_frame(&PeerFrame::AuthOk).expect("encode");
        let envelope: FrameEnvelope = serde_json::from_slice(&bytes).expect("parse envelope");
        assert_eq!(envelope.v, ENVELOPE_VERSION);
        assert_eq!(envelope.enc, EncScheme::None);
    }

    #[test]
    fn rejects_unsupported_envelope_version() {
        let envelope = FrameEnvelope {
            v: 99,
            enc: EncScheme::None,
            payload: serde_json::to_vec(&PeerFrame::AuthOk).unwrap(),
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let err = decode_frame(&bytes).expect_err("should reject");
        assert!(err.contains("unsupported envelope version"));
    }

    #[tokio::test]
    async fn length_delimited_round_trips_a_payload() {
        let mut buf: Vec<u8> = Vec::new();
        length_delimited::write(&mut buf, b"hello peer")
            .await
            .expect("write");
        let mut cursor = std::io::Cursor::new(buf);
        let read = length_delimited::read(&mut cursor)
            .await
            .expect("read")
            .expect("some payload");
        assert_eq!(read, b"hello peer");
    }

    #[tokio::test]
    async fn length_delimited_read_returns_none_on_clean_eof() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let read = length_delimited::read(&mut cursor).await.expect("read");
        assert!(read.is_none());
    }
}
