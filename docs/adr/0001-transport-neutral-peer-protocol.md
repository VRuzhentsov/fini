# 0001 — Transport-neutral peer protocol, pluggable transports, deferred encryption seam

## Status

Accepted. Implemented for `TcpWs` (network) and `Sim` (test/E2E) transports.
Real Bluetooth (Linux BlueZ + Android) is a stacked follow-up PR against
this same abstraction; LoRaWAN is a reserved, unimplemented `TransportKind`.

## Context

Issue #25 asked for Bluetooth as a second transport for `DeviceConnection`/
`SpaceSync`, alongside the existing network transport (mDNS/DNS-SD discovery
+ WebSocket sync), with full E2E coverage. The original protocol type was
named `WsMessage` and the pairing/auth gate lived directly inside the
WebSocket accept loop (`ws_server::handle_connection`) — the protocol and
the transport were the same code.

During planning this was scoped up: the app should support **pluggable
transport backends** (TCP-WebSocket today, Bluetooth next, LoRaWAN later)
carrying one **shared, transport-neutral peer protocol**, with **room left**
in the architecture for Signal-style end-to-end encryption (crypto itself
deferred). CI cannot exercise real radios (no Bluetooth hardware on
GitHub-hosted runners) or run two Android emulators reliably in one job, so
the test strategy needed a way to prove transport *selection and handoff
semantics* without real hardware.

## Decision

### Layering

```
Peer Protocol engine ── pair handshake · auth gate · sync/ack/bootstrap   (transport-neutral)
Peer Protocol codec ─── PeerFrame  <->  bytes                             (serde_json)
Frame envelope ──────── { v, enc, payload }                               (versioned, forward-compat)
Secure Channel ──────── encrypt/decrypt(bytes)  — seam: PlaintextChannel today, Signal later
Transport port ──────── Transport{kind,dial} · Link{kind,send,recv}       (opaque byte datagrams)
Transport adapters ──── TcpWs · Sim · [Bluetooth — PR B] · [LoRa — future]
```

- `WsMessage` was renamed to **`PeerFrame`**: it is the wire type of the
  peer protocol (pairing handshake + authenticated sync), not a WebSocket
  concept. `specs/space-sync/README.md` and `specs/device-connect/README.md`
  were updated from "websocket session" to "peer session over the selected
  transport" to match.
- The gate (`ws_server::handle_connection`'s pre-auth `PairRequest`/
  `PairAccept`/`PairComplete` handling plus the `Auth`→`AuthOk`/`AuthFail`
  check) moved out of the WebSocket-specific accept loop into
  `space_sync::session::run_peer_gate`, which operates on `PeerFrame` over a
  `Link` trait object. It is called identically by every adapter's accept
  loop (`transport::tcp_ws::run_server_on_port`,
  `transport::sim::run_server`). `space_sync::session::run_session` (the
  authenticated per-peer message loop) is likewise transport-neutral.
  `tokio_tungstenite`/`tungstenite::Message` are now confined to
  `transport::tcp_ws` — no other module depends on them.

### Why datagrams, not a byte stream or typed frames, at the `Link` boundary

- **Typed `PeerFrame` at the transport boundary** was rejected: it would
  pull the protocol codec into the transport and leave nowhere clean for
  the `SecureChannel` seam (the transport would see plaintext).
- **A raw `AsyncRead`/`AsyncWrite` byte stream** was rejected: WebSocket is
  already message-framed (`tokio-tungstenite` gives you a `Stream<Message>`,
  not a byte stream), so forcing it into a stream boundary would mean
  discarding and re-implementing framing tungstenite already does. BLE and
  LoRa are packet-oriented too, not streams.
- **Opaque byte datagrams** (`Link::send(Vec<u8>)`/`recv() -> Vec<u8>`,
  boundaries preserved) fit all three: the adapter owns chunking (WS
  message, future BLE MTU chunks, future LoRa packet), and the codec above
  it (`transport::codec`) only ever sees whole payloads.

### Sticky single-session handoff (no mid-session transport migration)

Acceptance criteria required "Bluetooth fallback works after network
failure without duplicating or losing sync events" and left session handoff
as an open question. The chosen invariant: **at most one authenticated
session may exist per peer at any time**
(`DeviceConnectionState::try_claim_session`/`release_session`). Selection is
network-first *at establishment only* (`transport::selection::select_dial_order`);
a live session — on whichever transport claimed it — is kept until it
drops, then the next establishment attempt re-applies network-first order.

This was chosen over two alternatives:
- **Preemptive handoff** (tear down a live fallback session and reconnect
  over network as soon as it recovers) would require transferring
  in-flight/un-acked events across transports without duplication — exactly
  the failure mode the acceptance criteria warn about.
- **Concurrent sessions with `event_id` dedup** would be more resilient to
  flapping connectivity but makes idempotent delivery a hard requirement
  everywhere, rather than something the invariant rules out structurally.

Because at most one session can exist, duplicated/lost events across
transports is impossible by construction, not by careful bookkeeping.
Verified in `services::transport::tests::sticky_single_session_rejects_a_concurrent_second_claim`.

### Deferred encryption, forward-compatible wire now

Crypto (Signal-style X3DH + Double Ratchet) is out of scope for this PR, but
the wire format changes needed to add it later without breaking already-
shipped devices are in now:
- A versioned frame envelope (`transport::envelope::FrameEnvelope { v, enc, payload }`).
- A `SecureChannel` trait (`transport::secure_channel`) with a
  `PlaintextChannel` (identity) implementation — the only one that exists.
- A reserved, optional `key_material` field on the pair-complete handshake
  payload (`skip_serializing_if = "Option::is_none"`, so it costs nothing
  on the wire while unused).

Enabling real encryption later means adding a new `SecureChannel` impl and
populating `key_material`/`EncScheme` — additive, not a wire break.

### Test strategy: `Sim` is a first-class adapter, not a mock

`transport::sim` implements the exact same `Transport`/`Link` port as
`tcp_ws` (loopback TCP + length-delimited framing instead of a WebSocket
upgrade), so exercising it runs the *real* `run_peer_gate`/`run_session`
code, not a stand-in. This is what lets CI prove transport-selection,
auth-gating, and sticky-handoff semantics deterministically on GitHub-hosted
runners, which cannot run real Bluetooth radios:
- Rust integration tests (`services::transport::tests`) run two peer engines
  over an in-process/loopback Sim link — these are the runtime-agnostic
  proof that the semantics hold for *any* pair of real or future
  transports, including android-linux, android-android, and windows-android
  (the peer protocol engine is identical Rust on every OS; there is no
  "android-ness" at this layer).
- The Playwright E2E suite adds a real-process, real-app-binary layer:
  `transport-selection.spec.ts` proves normal (network-available) actors
  claim `tcp_ws`; `peer-sync-over-sim.spec.ts` proves actors with the
  network transport genuinely disabled (`FINI_DISCOVERY_DISABLED=1`) claim
  `sim` instead and replicate an approved Space over it, with no new
  SpaceSync consent prompt.
- What this does **not** prove: a real Android runtime or real radio
  crossing devices. That requires local/device-lab verification
  (`make e2e-bt-local`, PR B) — GitHub-hosted CI has no Bluetooth hardware
  and pairing two Android emulators in one job is heavy/flaky. See
  `specs/e2e/transports.md` for the full topology-to-verification matrix.

### What was deliberately not built

- **No dynamic `Box<dyn Transport>` registry.** With two concrete adapters,
  production dial loops (`tcp_ws::spawn_dial_loop`,
  `sim::spawn_fallback_dial_loop`) call each adapter's functions directly.
  Both adapters do implement the `Transport` trait (proven in
  `services::transport::tests::both_adapters_satisfy_the_transport_port`),
  so a registry can be introduced without reshaping callers once a third
  adapter (real Bluetooth, then LoRaWAN) makes one worth its weight.
- **No autonomous discovery for `Sim`.** `tcp_ws` discovery reuses the
  existing mDNS/UDP presence worker; `Sim` peers are configured directly via
  `FINI_SIM_PEER_PORTS` (test/E2E only). The real Bluetooth adapter (PR B)
  needs genuine discovery (scanning OS-paired devices) and will use the
  reserved `Candidate` type for it.
- **Live UI wiring for the lifecycle bus.** `DeviceConnectionState`
  exposes `subscribe_lifecycle()` (a `tokio::sync::broadcast` of
  session-established/ended events) and `session_kind()`
  (exposed via the `device_connection_session_transport` command and used
  by the E2E specs), but the UI-facing `device_connection_transport_statuses`
  command still reports the static enabled/available/preferred heuristics
  from `device_connection::transport` (kept from the original draft) rather
  than the live session's transport. Wiring push-based live status into the
  UI is follow-up work, not required for this PR's CI/protocol scope.

## Consequences

- Adding a transport (Bluetooth now in PR B, LoRaWAN later) means writing
  one adapter module implementing `Transport`/`Link`; no change to
  `space_sync::session`, discovery types, or the codec.
- `WsMessage` no longer exists as a name; any external references, docs, or
  in-flight branches using it need updating.
- The network transport's gate/session code is shared with every future
  adapter — a bug fixed in `run_peer_gate`/`run_session` is fixed for all
  transports at once, not per-adapter.
