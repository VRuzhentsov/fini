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

    /// A reader that survives being cancelled mid-frame.
    ///
    /// `read` below is not cancellation-safe, and the session loop reads
    /// inside a `tokio::select!` -- so every ping it has to send and every
    /// outbox event it has to forward drops the in-flight read. Dropped
    /// after the four length bytes and before the payload, those four bytes
    /// are simply gone: the next read then takes the payload's first four
    /// bytes as a length. For our frames that is `{"v`, or 2065856034,
    /// which fails the size check and kills an authenticated session.
    ///
    /// This keeps whatever has arrived in a buffer that belongs to the
    /// link rather than to the future, and fills it with `read_buf`, which
    /// *is* cancellation-safe: if the future is dropped, nothing was taken
    /// from the socket that is not already in the buffer. Cancelling costs
    /// a wasted poll and nothing else.
    #[derive(Default)]
    pub struct FrameReader {
        pending: Vec<u8>,
    }

    impl FrameReader {
        /// One frame, or `None` at a clean EOF.
        pub async fn read<R: tokio::io::AsyncRead + Unpin>(
            &mut self,
            reader: &mut R,
        ) -> Option<Result<Option<Vec<u8>>, String>> {
            loop {
                match self.take_frame() {
                    Some(Ok(frame)) => return Some(Ok(Some(frame))),
                    Some(Err(err)) => return Some(Err(err)),
                    None => {}
                }
                match reader.read_buf(&mut self.pending).await {
                    Ok(0) => {
                        return if self.pending.is_empty() {
                            Some(Ok(None))
                        } else {
                            // A frame was promised and the socket closed
                            // inside it. Saying EOF here would report a
                            // clean shutdown for a truncated one.
                            Some(Err(format!(
                                "connection closed mid-frame with {} bytes buffered",
                                self.pending.len()
                            )))
                        }
                    }
                    Ok(_) => continue,
                    Err(err) => return Some(Err(format!("read frame: {err}"))),
                }
            }
        }

        fn take_frame(&mut self) -> Option<Result<Vec<u8>, String>> {
            if self.pending.len() < 4 {
                return None;
            }
            let len = u32::from_be_bytes([
                self.pending[0],
                self.pending[1],
                self.pending[2],
                self.pending[3],
            ]);
            if len > MAX_FRAME_LEN {
                return Some(Err(format!(
                    "frame length {len} exceeds max {MAX_FRAME_LEN}; first bytes were {:?}",
                    String::from_utf8_lossy(&self.pending[..4])
                )));
            }
            let total = 4 + len as usize;
            if self.pending.len() < total {
                return None;
            }
            let frame = self.pending[4..total].to_vec();
            self.pending.drain(..total);
            Some(Ok(frame))
        }
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
            // Show the bytes, not just the number they decoded to. A length
            // this wrong means the stream is not carrying length-prefixed
            // frames at all, and what it *is* carrying names the writer --
            // "{\"ty" reads very differently from an HTTP verb.
            return Err(format!(
                "frame length {len} exceeds max {MAX_FRAME_LEN}; first bytes were {:?}",
                String::from_utf8_lossy(&len_buf)
            ));
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

    /// A read that loses a `select!` race must not eat the bytes it had.
    ///
    /// This is the shape that killed every loopback session in the
    /// `actors-loopback` lane: the session loop reads inside a `select!`,
    /// so a ping to send or an event to forward drops the read future.
    /// With `read_exact` the four length bytes went with it, and the next
    /// read took the payload for a length -- `{"v`, two billion, session
    /// over. Here the read is cancelled between the length and the
    /// payload, deliberately, and the frame still has to arrive whole.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cancelled_read_keeps_the_bytes_it_already_took() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        let payload = b"{\"v\":1,\"enc\":\"none\",\"payload\":\"AAAA\"}".to_vec();

        // The length first, then a pause, then the body -- which is exactly
        // how a real socket delivers a frame that spans two packets.
        client
            .write_all(&(payload.len() as u32).to_be_bytes())
            .await
            .expect("write length");
        client.flush().await.expect("flush length");

        let mut reader = length_delimited::FrameReader::default();

        // Lose the race, repeatedly, while only the length is available.
        for _ in 0..5 {
            tokio::select! {
                _ = reader.read(&mut server) => panic!("no whole frame is available yet"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        client.write_all(&payload).await.expect("write payload");
        client.flush().await.expect("flush payload");

        let frame = reader
            .read(&mut server)
            .await
            .expect("a frame, not EOF")
            .expect("a readable frame")
            .expect("a payload");
        assert_eq!(frame, payload, "the frame must survive the cancellations");
    }

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
