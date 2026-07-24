# Context

Glossary for terms introduced or sharpened while adding pluggable transports
to `DeviceConnection`/`SpaceSync` (issue #25). See
`docs/adr/0001-transport-neutral-peer-protocol.md` for the decision record
behind these terms.

- **Peer Protocol** — the transport-neutral application protocol: pairing
  handshake (`PairRequest`/`PairAccept`/`PairComplete`) plus authenticated
  sync (`Auth`/`AuthOk`/`AuthFail`, `SyncEvent`, `Ack`, `BootstrapStart`/
  `BootstrapEnd`, `SpaceMappingUpdate`, `SpaceSyncEnd`). Implemented in
  `src-tauri/src/services/space_sync/session.rs`. Never tied to any one
  transport.

- **`PeerFrame`** — one message of the Peer Protocol
  (`space_sync::types::PeerFrame`). Renamed from `WsMessage`: the type
  carries pairing and sync, not just WebSocket traffic, and is carried by
  whichever `Transport`/`Link` is currently selected for a peer.

- **Transport** — a pluggable adapter that carries `PeerFrame`s between two
  devices (`services::transport::Transport` trait). Implementations:
  `tcp_ws` (network — mDNS/UDP discovery + WebSocket `Link`), `sim`
  (deterministic loopback-TCP adapter for tests/E2E), and reserved-but-
  unimplemented kinds `Bluetooth` (real BlueZ + Android, PR B) and `LoRa`
  (future). A transport only knows how to establish a link and move framed
  bytes — it has no pairing or sync semantics.

- **Link** — one live, point-to-point connection to a peer over a specific
  transport (`services::transport::Link` trait). Moves opaque byte
  datagrams (`send(Vec<u8>)`/`recv() -> Vec<u8>`, boundaries preserved);
  each adapter owns its own chunking/framing.

- **Candidate** — an untrusted peer surfaced by a transport's discovery
  step. Never confers trust by itself; only a successful `PeerFrame::Auth`
  handshake over a dialed `Link` does.

- **Secure Channel** — the encryption seam between the `PeerFrame` codec and
  a `Link` (`services::transport::secure_channel::SecureChannel` trait).
  `PlaintextChannel` (identity, no-op) is the only implementation today;
  reserved for a future Signal-style (X3DH + Double Ratchet) implementation.
  Crypto itself is out of scope for issue #25 — see [[frame-envelope]].

- **Frame envelope** — the versioned wrapper around every encoded
  `PeerFrame` (`services::transport::envelope::FrameEnvelope { v, enc, payload }`).
  Exists so enabling real encryption later is an additive wire change
  (`enc` moves off `EncScheme::None`), not a breaking one.

- **Session** — the authenticated, per-peer message loop after a successful
  `Auth` handshake (`space_sync::session::run_session`). At most one session
  may be claimed per peer at any time, regardless of transport — see
  **sticky handoff**.

- **Transport selection** — choosing which transport a new session
  establishes over. Network-first at establishment only
  (`services::transport::selection::select_dial_order`): try `TcpWs`,
  fall back to a secondary role (`Sim` in this PR; `Bluetooth` once PR B
  lands) only when the network transport is unavailable for that peer.

- **Sticky handoff** — the invariant that a live session is never migrated
  to a different transport mid-session
  (`DeviceConnectionState::try_claim_session`/`release_session`). A session
  persists on whichever transport claimed it until it drops; the *next*
  establishment attempt re-applies network-first selection. This is what
  makes duplicated or lost sync events across transports structurally
  impossible, not just unlikely.

- **Connection Manager** — informal name for the selection + claim/release
  logic collectively (`services::transport::selection` +
  `DeviceConnectionState`'s session-claim methods), not a single struct in
  this PR. A `tokio::sync::broadcast` lifecycle bus
  (`DeviceConnectionState::subscribe_lifecycle`) fans out session
  established/ended events for future UI consumption.
