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
//! A **channel** is the configured connection between two devices; a
//! **channel kind** is the medium it uses (`network`, `bluetooth`).
//! "Transport" is the low-level synonym, kept in this module's internals for
//! the adapter that carries a link — see `docs/naming.md`.

pub mod channel;
pub mod pairing;
pub mod sync;
