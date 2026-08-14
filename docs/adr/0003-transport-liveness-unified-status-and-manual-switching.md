# 0003 — Transport liveness, a unified per-row status model, and manual transport switching

## Status

Proposed. Three phases, in dependency order:

1. **Liveness detection** — a WebSocket-native ping/pong heartbeat, self-
   contained inside the Network `Link` implementation, so a silently-dead
   link (peer out of range, killed process, a NAT/firewall dropping the
   connection with no RST) is detected promptly instead of never, or only
   after an OS-level timeout. Bluetooth already has an equivalent
   mechanism, verified below — it needs no changes.
2. **Unified status state machine + push-based UI updates** — replace each
   transport row's ad hoc `available`/`connected`/`detail` computation with
   one shared four-state model (`Unconfigured` / `Configured, unreliable` /
   `Configured, reliable` / `Live`), driven off real failure history and
   liveness, and pushed to the UI immediately instead of only discovered on
   the next 5s poll.
3. **Manual transport preference** — click either row to force this pair
   onto that transport now, persisted so it also governs future automatic
   reconnects, with the same preference propagated to the peer.

## Context

ADR-0002 Phase 2 added `TransportStatus.connected`, computed from
`DeviceConnectionState::session_kind(peer_id) == Some(kind)` — "is a
session live on this transport right now." That flag is only as good as
two things neither existed yet:

1. **Whether a claimed session is actually still alive.** `run_session`'s
   loop only calls `release_session` when the underlying transport read
   errors or returns EOF (`session.rs`, the `tokio::select!` loop around
   `recv_frame`). Nothing times out an idle read, and nothing periodically
   confirms the peer is still there — for the **Network** transport. A
   silent death there (NAT/firewall dropping the connection with no RST,
   peer process killed) leaves the session marked `connected: true`
   indefinitely, until (if ever) the OS's own default TCP timeout fires,
   which can be tens of minutes to hours unless tuned. **Bluetooth does
   not have this gap**: `ble_gatt::datagram::DatagramChannel::recv()`
   already documents and implements returning `None` "which includes the
   peer vanishing without warning, not just an orderly `close()`" — backed
   by real OS-level GATT connection-state callbacks (BlueZ D-Bus on Linux,
   `BluetoothGattCallback` on Android), and `transport::ble::BleLink::recv()`
   already forwards that `None` straight through. Verified by reading
   `ble-gatt`'s pinned source, not assumed — see "What already exists"
   below.
2. **Whether the UI actually learns about a change promptly.**
   `DeviceConnectionState` already has a `subscribe_lifecycle()` broadcast
   of `SessionEstablished`/`SessionEnded`, built in ADR-0001 and explicitly
   marked `#[allow(dead_code)]`: "Reserved for UI consumption ... wiring a
   push-based subscriber is follow-up work." That follow-up never
   happened; `DeviceView.vue` only ever learns `connected` via
   `refreshLiveConnectedState`'s 5s poll (`TRANSPORT_STATUS_POLL_INTERVAL_MS`).

Together these produced the actual user-visible symptom that prompted this
ADR: the Bluetooth row's green "connected now" state felt unreliable —
sometimes stuck stale, sometimes slow to appear. Detection itself was
already reliable there (see above); the gap was entirely (2) — a real
`SessionEnded` fires the instant `ble-gatt` notices the disconnect, but
nothing pushed that to the UI faster than the next 5s poll.

Separately, the existing amber "Available for fallback" state
(`bluetooth_status_detail`) conflates two genuinely different situations
under one color: "fully configured, no reason to distrust it, just not the
one carrying the live session because network is preferred" (the common
case for any two devices sharing a network — arguably the *majority* of
the time for most pairs) and "configured, but recent attempts to actually
use it have failed" (genuinely worth flagging). Reading the first case as
amber — visually a caution/warning color to most users — is what made the
status row feel like something was wrong essentially all the time, even
when nothing was.

### What already exists and is reused, unchanged

- `session_kind()`, `try_claim_session()`, `release_session()`
  (`DeviceConnectionState`) — the session-claim bookkeeping itself is
  correct and untouched; only *when* `release_session` fires changes
  (Phase 1 adds a new, additional trigger for it).
- `subscribe_lifecycle()` / `LifecycleEvent` (`transport::selection`) — the
  broadcast bus already exists and already fires at the right moments;
  Phase 2 is exactly the "wiring a push-based subscriber" ADR-0001 deferred.
- `NETWORK_UNRESPONSIVE_THRESHOLD` / `tcp_dial_failure_count` /
  `network_effectively_available()` (`transport::selection`,
  `DeviceConnectionState`) — Network already tracks exactly the kind of
  "recent failure history" signal Phase 2 needs for Bluetooth too; that
  pattern is extended, not reinvented.
- `select_dial_order()` (`transport::selection`) — the network-first dial
  order function Phase 3 extends with an optional preference override.
- `run_session`'s single shared loop (`space_sync/session.rs`) already
  runs identically for `TcpWs`, `Bluetooth`, and `Sim`, and already treats
  any `recv_frame` returning `None`/`Err` as "dead, clean up" — Phase 1
  needs zero changes here (see "Architecture layering" below).
- `ble_gatt::datagram::DatagramChannel::recv()` (`ble-gatt`, pinned rev
  `af4ed1e`) — already closes on real, unsolicited peer loss, not just an
  orderly close. Its own doc comment states the guarantee directly; traced
  through `datagram/mod.rs`'s peripheral/central event loop, which listens
  for `GattEvent::Disconnected` and drops the affected peer's channel
  sender on it. `transport::ble::BleLink::recv()` already forwards this
  (`self.channel.recv().await?`). Nothing in Phase 1 touches Bluetooth.
- `tungstenite` (`tcp_ws.rs`'s underlying WS library) already auto-answers
  an incoming `Message::Ping` with a `Message::Pong` at the protocol level
  (`protocol/mod.rs`: `self.set_additional(Frame::pong(...))`) and surfaces
  both as real `Message` values through the read stream. Phase 1 only
  needs to *initiate* periodic pings and track staleness — the reply side
  is already handled by the library.

## Decision

### Phase 1 — WebSocket-native ping/pong, self-contained in `TcpWsLink`

**Architecture layering.** The two transports must present the *same*
liveness contract upward while using genuinely different, protocol-native
mechanisms underneath — not one mechanism forced uniformly onto both:

```
 App logic        run_session (space_sync/session.rs)
                   ── unchanged. Already: recv_frame() -> None/Err ⇒
                      release_session() + SessionEnded. No transport
                      knowledge, no liveness knowledge, at this layer.
                              ▲                          ▲
                     recv() -> None               recv() -> None
                              │                          │
 Link trait        ──────────┴──────────────────────────┴──────────
                    the uniform contract: "None means dead," for
                    whatever transport-specific reason.
                              │                          │
 Link impls        BleLink::recv()              TcpWsLink::recv()
                    (transport/ble.rs)           (transport/tcp_ws.rs)
                    already forwards             gains internal ping/pong
                    DatagramChannel's             bookkeeping (Phase 1's
                    None — no change              only real change)
                              │                          │
 Protocol/OS       ble-gatt's GATT event         WebSocket's own RFC 6455
 primitives         loop (GattEvent::             Ping/Pong control frames
                    Disconnected, from            (tungstenite auto-replies
                    BlueZ D-Bus / Android          incoming Pings; this
                    BluetoothGattCallback)         layer only needs to send
                                                    outbound Pings and time
                                                    out on missing Pongs)
```

Each protocol keeps its own "superiority" (GATT's native connection-state
callback for Bluetooth; WS's native control-frame ping/pong for Network) —
neither is reimplemented at the app layer, and `run_session` never learns
which one is in play. This is a smaller, more precise Phase 1 than treating
liveness as an application-level concern: no new `PeerFrame` variant, no
protocol-version gating question, no `run_session` changes at all.

**The actual change**, entirely inside `TcpWsLink` (`tcp_ws.rs`):

- `recv()`'s loop gains a `tokio::select!` arm alongside `self.source.next()`:
  a `tokio::time::interval` firing every `PING_INTERVAL` (15s) that sends
  `Message::Ping(vec![])` via `self.sink` and increments a miss counter.
- The existing `Some(Ok(_)) => continue` catch-all (today silently
  swallowing `Message::Ping`/`Message::Pong` alike) splits: an inbound
  `Message::Ping` still falls through to `continue` (tungstenite already
  queued the reply — nothing for this code to do), but an inbound
  `Message::Pong` resets the miss counter to 0 before continuing.
- After `PING_MISS_LIMIT` (3) consecutive missed intervals (~45s), `recv()`
  returns `None` itself — the same value a real read error already
  produces, so nothing downstream needs a new case.

15s/3-miss trades UI responsiveness for radio/battery cost — chosen with
Bluetooth in mind even though this phase's actual code change is
Network-only, so the same cadence is available later if a transport-level
mechanism ever needs revisiting.

`SimLink` (`sim.rs`, test/E2E-only — stands in for Bluetooth in
`actors-sim`) is plain TCP with no keepalive either, and shares Network's
gap in principle. Left alone here: a controlled test process either closes
its socket explicitly or is killed (which the OS turns into a real
FIN/RST), so the silent-death scenario this phase targets doesn't
organically occur in that harness. Revisit only if that stops holding.

### Phase 2 — Unified status state machine + push-based UI updates

**State machine** (`device_connection::transport`), replacing today's
`available`/`connected`/`detail` computation for both rows:

```rust
enum RowState {
    Unconfigured { reason: String },
    Configured { reliable: bool },
    Live,
}
```

Evaluated in this order, per row:

1. `Unconfigured` — local preconditions not met. Unchanged sub-reasons:
   Bluetooth (`disabled` / `no address stored` / `not OS-paired`), Network
   (`not discovered`/`unreachable`). → gray dot.
2. `Live` — `session_kind(peer_id) == Some(kind)` for this row. Trustworthy
   as of Phase 1: `session_kind` only ever reflects a session
   `run_session` hasn't yet torn down, and Phase 1 is what makes sure a
   silently-dead `Link` (whichever transport-specific mechanism detected
   it) gets torn down promptly instead of lingering. → green dot **with a
   border** (the row's sole "this is the one actually carrying traffic
   right now" signal — replaces today's separate "connected now" text
   badge, which is dropped as redundant with the border).
3. `Configured { reliable: true }` — configured, not live right now, and
   *not* had `>= UNRESPONSIVE_THRESHOLD` (3, matching
   `NETWORK_UNRESPONSIVE_THRESHOLD`'s existing value) consecutive recent
   dial/auth failures. A fresh pair with zero attempts yet counts as
   reliable by default — "no reason to distrust it" is the bar, not "has
   been proven to work," matching how `network_effectively_available`
   already treats an untested peer. → green dot, no border.
4. `Configured { reliable: false }` — configured, but recent consecutive
   failures are at or above the threshold. → amber dot. This is the *only*
   remaining use of amber, and it now means something worth a user's
   attention ("this is set up, but hasn't been working"), not "network
   just happens to be doing the job instead."

Bluetooth needs a new `bluetooth_dial_failure_count` /
`bluetooth_effectively_reliable()` pair on `DeviceConnectionState`,
structurally identical to the existing `tcp_dial_failure_count` /
`network_effectively_available()` — incremented on a failed BLE dial/auth
attempt, reset to 0 on a successful one (mirroring the existing
`tcp_failure_count_resets_after_a_sim_session_ends` test's pattern).

**Push-based updates:** `lib.rs`'s app setup subscribes to
`subscribe_lifecycle()` once and forwards each `SessionEstablished`/
`SessionEnded` event to the frontend via `app.emit("device-connection://session-changed", ...)` (payload: `peer_device_id`, `kind`, `established: bool`).
`device.ts` listens for this event and calls the existing
`refreshLiveConnectedState`/`refreshTransportStatuses` immediately on
receipt, rather than waiting for the next poll tick. The 5s poll
(`TRANSPORT_STATUS_POLL_INTERVAL_MS`) stays as a correctness safety net —
push is a latency improvement, not the sole source of truth, so a missed
or dropped event self-heals within 5s instead of staying wrong
indefinitely.

**UI**: `DeviceView.vue`'s dot binding changes from the current three-way
`connected ? green : available ? amber : gray` to a four-way switch over
the new `RowState`, with the border applied only for `Live`. The
`preferred` text badge (today: "this is what the automatic dial order
would pick") stays as a **separate, independent signal** from the
border — the two can legitimately disagree (e.g., pinned to Bluetooth via
Phase 3 while Network is `Configured{reliable:true}` and would be dialed
automatically absent that pin) — see Phase 3.

### Phase 3 — Manual transport preference

New column on `paired_devices`: `preferred_transport TEXT NULL` (stores
`"tcp_ws"`/`"bluetooth"`, matching `TransportKind`'s existing
`#[serde(rename_all = "snake_case")]`; `NULL` = no manual preference, pure
automatic network-first behavior, today's default for every existing row).

**Clicking a row** (available on both rows, any time that row's state
isn't `Unconfigured` — including before any session has ever existed for
the pair, since setting the preference doesn't require anything to switch
away from):

1. Persists `preferred_transport` for that peer locally.
2. If a session is currently live on a *different* transport, forces it
   closed (a new, explicit teardown path — deliberately breaking
   ADR-0001's documented sticky-handoff invariant for this one
   user-initiated case only; every other establishment path keeps that
   invariant unchanged) and immediately attempts to (re)dial on the
   clicked transport.
3. Sends the peer `PeerFrame::SwitchTransport { to: TransportKind, requested_at: String }`
   (RFC3339 timestamp) over the *current* session if one exists — this is
   an imperative notification, not a negotiated request; there is no
   accept/reject frame.

**Receiving `SwitchTransport`:** the peer updates its own
`preferred_transport` to match and attempts the same local force-switch.
If it can't (the target transport isn't `Configured`/`Live`-capable on its
end — e.g., Bluetooth not OS-paired there), it does nothing further: the
existing session, whatever it is, simply continues. No error is surfaced
back to the initiator over the wire; the initiator's own dial attempt on
its chosen transport will independently fail/time out on its own merits,
and its UI reflects that failure the normal way (the clicked row not
reaching `Live`).

**Race resolution:** if both sides send conflicting `SwitchTransport`
frames close together, the one carrying the later `requested_at` wins on
both sides — the side that receives a frame with a *later* timestamp than
its own most recent local switch adopts it (updates its own
`preferred_transport` to match and re-attempts accordingly); a frame with
an *earlier* timestamp than the receiver's own most recent switch is
ignored. No clock-sync mechanism beyond each device's own system clock is
introduced; a true simultaneous double-click within clock-skew tolerance
is an accepted, rare edge case, not specifically handled beyond
last-writer-wins.

**`select_dial_order` extension:** establishment (never mid-session, per
the unchanged sticky-handoff rule for every path except the manual
force-switch above) checks `preferred_transport` first — if set and that
transport is currently viable, it's the sole/first entry in the dial
order; otherwise falls through to today's unchanged network-first
`select_dial_order(network_available, fallback_available)` behavior.

## Consequences

- Phase 1 adds no wire protocol at all — `Message::Ping`/`Pong` are
  WebSocket control frames handled entirely below `Link`, invisible to
  `PeerFrame`/`recv_frame`/an old peer's protocol version. An old Fini
  build on the other end of a `TcpWsLink` still responds to WS-level
  `Ping`s exactly the same (that's `tungstenite`'s job, unrelated to
  Fini's own protocol versioning), so this phase is safe against every
  peer, old or new, automatically. Phase 3's `SwitchTransport` frame is
  the one addition in this ADR that *does* need the treatment
  `BluetoothAddressUpdate` got in ADR-0002: gated on `peer_protocol_version`
  learned during `Auth`/`AuthOk`, never sent proactively to a peer that
  hasn't proven it understands it.
- Phase 1 changes real behavior for every existing **Network** session
  (Bluetooth is already correct, unchanged): a currently-live TcpWs
  session that goes silent will now be torn down within ~45s instead of
  persisting indefinitely. This is the intended fix, not a side effect to
  guard against.
- Phase 3 is a genuine, deliberate exception to ADR-0001's sticky-handoff
  invariant ("selection only happens at session establishment ... kept
  until it drops") — worth flagging explicitly since that invariant is
  documented and tested (`transport::selection`'s module doc comment,
  `select_dial_order`'s test suite) as a load-bearing simplification
  ("duplicated or lost sync events structurally impossible"). The
  exception is scoped as narrowly as possible: only a manual,
  user-initiated click ever tears down a live session outside of a real
  transport failure.
- Phase 2's `bluetooth_effectively_reliable()` reuses `paired_devices`
  metadata already loaded per status check; no additional DB round-trip
  beyond what `device_connection_transport_statuses_impl` already does.
- No change to `specs/device-connect/README.md`'s trust model: pairing
  and disable/unpair semantics are untouched by this ADR. `Ping`/`Pong`
  and `SwitchTransport` are only ever exchanged on an already-authenticated
  session (post-`AuthOk`), same trust level as `SyncEvent`/`Ack`, not the
  pre-auth discovery-metadata tier `PairRequest`/`DiscoveryHello` sit at.
