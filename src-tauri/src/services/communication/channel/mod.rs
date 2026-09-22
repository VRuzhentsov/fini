//! Carrying bytes over a pair's channels.
//!
//! A **channel (transport)** is what a pair configured — Network or
//! Bluetooth, the two `ChannelKind`s. This module holds the connection code
//! underneath one: `tcp_ws` is how the Network channel actually connects,
//! `ble` is how the Bluetooth one does. Connection code is not itself a
//! channel and can be shared by several — see `../README.md`.
//!
//! `pairing`/`sync` speak one shared application protocol
//! (`crate::services::communication::sync::types::PeerFrame`) over whichever
//! `DataLink` is currently selected for a peer. This module defines that
//! boundary plus the connection code that implements it:
//!
//! - `tcp_ws` — how the Network channel connects (mDNS/UDP discovery +
//!   WebSocket link).
//! - `ble` — how the Bluetooth channel connects (Linux BlueZ, Android GATT,
//!   both via `ble-gatt`; see that module's doc comment).
//!
//! A `DataLink` moves opaque byte datagrams (whole payloads, boundaries
//! preserved); each adapter owns its own chunking/framing. Above `DataLink` sits
//! `codec` (envelope + `PeerFrame` (de)serialization) and `encryption`
//! (currently pass-through; the seam for future end-to-end encryption).

#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod ble;
pub mod codec;
pub mod encryption;
pub mod envelope;
pub mod radio;
pub mod selection;
pub mod service;
pub mod tcp_ws;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::services::communication::sync::types::PeerFrame;

/// Which channel carried (or could carry) a peer link.
///
/// One variant per channel, and one session slot per variant. It used to
/// carry two more: `Sim`, which was really an implementation of the
/// Bluetooth channel rather than a channel of its own (see `radio`), and
/// `LoRa`, reserved for an adapter nobody wrote. Making channels data
/// removed the reason to reserve anything here — a new channel is a
/// `channel_kinds` row and a `DataLink`, not an enum variant.
///
/// There used to be a second enum saying this same thing — `TransportKind`,
/// whose `TcpWs` variant was this one's `Network` — with a `From` impl
/// between them. Two names for one idea is exactly what the vocabulary work
/// set out to remove: a channel and a transport are one concept, and
/// `tcp_ws` named a protocol where a channel belongs. Collapsing them is
/// what makes that true in the code rather than only in the glossary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// mDNS/UDP presence + a WebSocket link.
    #[default]
    Network,
    /// GATT, via `ble-gatt`.
    Bluetooth,
}

impl ChannelKind {
    /// How this kind is stored: `channels.channel_kind`, seeded into
    /// `channel_kinds.code`. Deliberately the same strings serde produces,
    /// so the wire form and the stored form never diverge.
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

/// An untrusted candidate peer surfaced by a transport's discovery step.
/// Never confers trust by itself — only a successful `PeerFrame::Auth`
/// handshake over a dialed `DataLink` does. Not yet consumed by a runtime
/// registry (see `Transport` doc comment) — reserved for the real
/// Bluetooth adapter's discovery step (PR B), which unlike `tcp_ws`
/// (backed by the existing presence worker) and `sim` (statically
/// configured) needs an actual candidate list.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Candidate {
    pub peer_device_id: String,
    pub kind: ChannelKind,
    pub addr: String,
    pub port: u16,
}

/// A live, point-to-point connection to one peer. Moves opaque byte
/// datagrams; framing/chunking is the adapter's concern, not the caller's.
#[async_trait]
pub trait DataLink: Send {
    fn kind(&self) -> ChannelKind;
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

/// One piece of connection code for a `ChannelKind`.
/// `tcp_ws::TcpWsTransport` implements it. The production dial loop calls
/// its functions directly rather than going through a dynamic
/// `Box<dyn Transport>` registry; a registry becomes worth its weight once
/// a channel arrives that this list cannot name.
#[async_trait]
#[allow(dead_code)]
pub trait Transport: Send + Sync {
    fn kind(&self) -> ChannelKind;
    fn dial(&self, peer_device_id: &str, addr: &str, port: u16) -> BoxDialFuture;
}

#[allow(dead_code)]
pub type BoxDialFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Box<dyn DataLink>, String>> + Send>>;

/// Send one `PeerFrame` over a link (encode via `codec`, transport carries bytes).
pub async fn send_frame(link: &mut dyn DataLink, frame: &PeerFrame) -> Result<(), String> {
    let bytes = codec::encode_frame(frame)?;
    link.send(bytes).await
}

/// Receive one `PeerFrame` from a link (decode via `codec`).
/// `None` means the link closed; `Some(Err(_))` means a malformed/unreadable frame.
pub async fn recv_frame(link: &mut dyn DataLink) -> Option<Result<PeerFrame, String>> {
    match link.recv().await? {
        Ok(bytes) => Some(codec::decode_frame(&bytes)),
        Err(err) => Some(Err(err)),
    }
}
