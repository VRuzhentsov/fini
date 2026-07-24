//! Transport-neutral peer link abstraction.
//!
//! `DeviceConnection`/`SpaceSync` speak one shared application protocol
//! (`crate::services::space_sync::types::PeerFrame`) over whichever
//! `Transport`/`Link` is currently selected for a peer. This module defines
//! that boundary plus the adapters that implement it:
//!
//! - `tcp_ws` — the network transport (mDNS/UDP discovery + WebSocket link).
//! - `sim` — a deterministic, CI-safe adapter used by tests and E2E to
//!   exercise transport selection/fallback/handoff without real radios.
//! - Bluetooth (Linux BlueZ + Android) and LoRaWAN are reserved `TransportKind`
//!   variants with no adapter yet; see `docs/adr/0001-transport-neutral-peer-protocol.md`.
//!
//! A `Link` moves opaque byte datagrams (whole payloads, boundaries
//! preserved); each adapter owns its own chunking/framing. Above `Link` sits
//! `codec` (envelope + `PeerFrame` (de)serialization) and `secure_channel`
//! (currently pass-through; the seam for future end-to-end encryption).

pub mod codec;
pub mod envelope;
pub mod secure_channel;
pub mod selection;
pub mod sim;
pub mod tcp_ws;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::services::space_sync::types::PeerFrame;

/// Which adapter carried (or could carry) a peer link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    /// mDNS/UDP discovery + WebSocket sync (the network transport).
    TcpWs,
    /// Deterministic loopback-TCP adapter used by tests/E2E to prove
    /// selection, fallback, and handoff without a real radio.
    Sim,
    /// Reserved for the real Bluetooth adapter (Linux BlueZ + Android),
    /// landing in a follow-up PR. No adapter implements this kind yet.
    Bluetooth,
    /// Reserved for a future LoRaWAN adapter. No adapter implements this
    /// kind yet.
    LoRa,
}

/// An untrusted candidate peer surfaced by a transport's discovery step.
/// Never confers trust by itself — only a successful `PeerFrame::Auth`
/// handshake over a dialed `Link` does. Not yet consumed by a runtime
/// registry (see `Transport` doc comment) — reserved for the real
/// Bluetooth adapter's discovery step (PR B), which unlike `tcp_ws`
/// (backed by the existing presence worker) and `sim` (statically
/// configured) needs an actual candidate list.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Candidate {
    pub peer_device_id: String,
    pub kind: TransportKind,
    pub addr: String,
    pub port: u16,
}

/// A live, point-to-point connection to one peer. Moves opaque byte
/// datagrams; framing/chunking is the adapter's concern, not the caller's.
#[async_trait]
pub trait Link: Send {
    fn kind(&self) -> TransportKind;
    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String>;
    /// `None` means the link closed (peer disconnected or read error).
    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>>;
    /// The peer's address, if the transport has one at the byte-stream
    /// level (network transports do; used by `run_peer_gate` as the
    /// `from_addr` stored with a pre-auth `PairRequest` so the receiver can
    /// address its `PairAccept`/`PairComplete` reply). `None` for transports
    /// without a meaningful notion of address (or where it isn't known).
    fn peer_addr(&self) -> Option<String> {
        None
    }
}

/// One adapter implementing a `TransportKind`. `tcp_ws::TcpWsTransport` and
/// `sim::SimTransport` both implement this — proven polymorphically in
/// `transport::tests` — but with only two concrete adapters in this PR,
/// production dial loops (`tcp_ws::spawn_dial_loop`,
/// `sim::spawn_fallback_dial_loop`) call each adapter's functions directly
/// rather than through a dynamic `Box<dyn Transport>` registry. A registry
/// becomes worth its weight once a third adapter (real Bluetooth, then
/// LoRaWAN) lands.
#[async_trait]
#[allow(dead_code)]
pub trait Transport: Send + Sync {
    fn kind(&self) -> TransportKind;
    fn dial(&self, peer_device_id: &str, addr: &str, port: u16) -> BoxDialFuture;
}

#[allow(dead_code)]
pub type BoxDialFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Box<dyn Link>, String>> + Send>>;

/// Send one `PeerFrame` over a link (encode via `codec`, transport carries bytes).
pub async fn send_frame(link: &mut dyn Link, frame: &PeerFrame) -> Result<(), String> {
    let bytes = codec::encode_frame(frame)?;
    link.send(bytes).await
}

/// Receive one `PeerFrame` from a link (decode via `codec`).
/// `None` means the link closed; `Some(Err(_))` means a malformed/unreadable frame.
pub async fn recv_frame(link: &mut dyn Link) -> Option<Result<PeerFrame, String>> {
    match link.recv().await? {
        Ok(bytes) => Some(codec::decode_frame(&bytes)),
        Err(err) => Some(Err(err)),
    }
}
