//! Everything two paired devices do to reach each other.
//!
//! Three concerns, in the order a pair meets them:
//!
//! - `pairing` — establishing trust between two devices, and the channels a
//!   pair has configured since.
//! - `channel` — carrying bytes over one of those channels. Holds the
//!   adapters (`tcp_ws`, `ble`, `sim`), the framing, and the encryption seam.
//! - `sync` — the application protocol that runs over a channel, and the
//!   outbox that survives a channel being down.
//!
//! A **channel (transport)** is the configured path between two devices —
//! one concept, two words, the second being the older networking one. A
//! **channel kind** is which one it is: `network` or `bluetooth`.
//!
//! See `README.md` beside this file for how `Channel`, `PeerSession`,
//! `DataLink` and `PeerFrame` differ, and `docs/glossary.md` for what each
//! word means.

pub mod channel;
pub mod pairing;
pub mod sync;
