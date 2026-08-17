# 0003 — Transport liveness, a unified per-row status model, and manual transport switching

## Status

Phases 1–3 shipped in v0.3.0. **Phases 2 and 3 are superseded by the
"Revision" section below**, added after a real-world bug surfaced post-
release: two paired devices could show different colors (amber on one,
green on the other) for what should have been one shared, symmetric
connection. The revision keeps Phase 1 (WebSocket-native ping/pong)
unchanged and replaces Phase 2's failure-counter reliability model and
Phase 3's `SwitchTransport` negotiation outright — read Phases 2/3 below
for the historical reasoning that motivated the original per-row state
machine and manual-pin feature, then the Revision section for what
actually ships now. Three original phases, in dependency order:

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

## Revision — dual-connection liveness, primary transport, and error codes

### The bug

Phase 2's `Configured { reliable }` bit came from `tcp_dial_failure_count`/
`bluetooth_dial_failure_count` — counters incremented only by whichever
side actually *dials* a given transport for a given peer (the deterministic
dialer rule: `my_id < peer_id` dials, the other side only ever accepts).
The accepting side never dials that peer on that transport, so its own
failure count for it is permanently zero — it has no way to ever observe
the other side's dial failures, or lack thereof. Two peers of the same
pair were therefore structurally guaranteed to be able to disagree on
whether a transport was "reliable": Android (a Bluetooth central, dialing)
could accumulate real failures and read amber, while Linux (the peripheral,
only ever accepting) stayed permanently "reliable" and read green — for
the *same* physical link. Sticky handoff made this worse in the meantime,
too: `Live`'s definition (`session_kind(peer_id) == Some(kind)`) reflects
whichever transport happened to win session establishment, not any
ongoing proof that it's still healthy in both directions.

### The fix: stop tracking dial failures, prove liveness directly — per transport, continuously

The root cause is that `reliable` was inferring health from an indirect,
asymmetric side-channel (dial outcomes) instead of the two peers directly
proving to each other, right now, that traffic flows both ways. Fixing
just the asymmetry (e.g. sharing failure counts over the wire) would still
leave a *sticky* signal — proven once, trusted indefinitely — which is a
weaker guarantee than the color implies. The revision replaces the
failure-counter model with a continuously-reproven, symmetric,
per-transport bidirectional ping/ack proof, and removes the "one live
session per peer" assumption (ADR-0001's sticky-handoff invariant) that
made a single `Live` bit meaningful in the first place: **both Network and
Bluetooth now stay connected to a peer simultaneously**, each independently
gray → amber → green, with a separate "primary" flag marking which one
carries real application traffic.

**Per-(peer, transport) session claiming**, not per-peer. `try_claim_session`
now succeeds independently for each transport — a live Bluetooth session no
longer blocks Network from also being claimed for the same peer, and vice
versa. Each transport's own dial loop (`tcp_ws::spawn_dial_loop`,
`ble::spawn_dial_loop`, `sim::spawn_fallback_dial_loop`) now dials
unconditionally whenever it doesn't already have a session with a peer on
*its own* transport — no more standing down because the other transport is
already connected, or because the pair is pinned to the other transport.
This is what "maintain a real parallel connection on standby" means: the
non-primary transport isn't merely capped at amber and left idle, it's a
fully live, continuously-proven connection that happens not to carry
application traffic right now.

**Bidirectional ping/ack proof** (`PeerFrame::Ping`/`Pong`, replacing dial
failure counts as the reliability signal): every connected transport
exchanges an app-level `Ping` every 15s (`session::APP_PING_INTERVAL`,
matching Phase 1's WS-native cadence for consistency, though this is a
separate layer on top of it — Phase 1's ping stays, doing the same job it
always did, detecting a silently-dead `Link` at the transport level; this
new one proves the *application* protocol is flowing, symmetrically, in
both directions). `TransportAckState` (`device_connection::types`) tracks,
per (peer, transport): `own_ping_acked` (this device's own outbound ping
was answered) and `peer_ping_received` (this device received and answered
an inbound ping from the peer). **Green requires both true, right now** —
"continuously re-proven," not "proven once and trusted until something
explicitly says otherwise": 3 consecutive missed cycles on either side
(reusing Phase 1's 15s/3-miss shape) flips the corresponding flag back to
false, with no separate signal required to notice the lapse. Both peers
run the identical protocol at the same cadence, so a healthy link
converges both sides to green within about one interval of each other —
there is no scenario where one peer can be stuck green while its actual
peer reads amber for the same link, because neither side's green depends
on anything the other side alone controls.

**"Primary" transport** replaces `Live`/the old sticky "which transport is
the session on" concept. Recomputed automatically after every claim or
release (`DeviceConnectionState::recompute_primary_locked`): Network wins
whenever it's connected, unless the pair is pinned to Bluetooth (and
Bluetooth is connected), in which case Bluetooth wins — otherwise whichever
transport is connected wins, or neither if none is. This is a pure function
of current connection state and the stored pin, not something that can
drift or need manual invalidation, and it reuses the *exact* selection
rule Phase 3 originally used for dial ordering ("Same rule as today, just
relabeled") — the only change is what it's selecting between (an
already-connected transport to treat as primary, not a transport to dial
next).

**`SwitchTransport` negotiation is gone, not adapted.** Phase 3's whole
negotiation machinery — the wire frame, `TransportPreferenceAdoption`,
`adopt_peer_transport_preference`, the last-writer-wins timestamp race, the
run_session startup relay — existed to force-close a live session on one
transport and (re)dial the other, because only one session could ever be
live at a time. With both transports always connected regardless of the
pin, there's nothing left to negotiate or force-close: a pin change
(`device_connection_set_preferred_transport_impl`) just persists the new
`preferred_transport` value and calls
`DeviceConnectionState::refresh_primary`, which re-runs the same selection
rule immediately. **ADR-0001's sticky-handoff invariant is fully retired**,
not narrowly excepted the way Phase 3 treated it — "at most one
authenticated session live per peer" was already false the moment both
transports connect independently; nothing in the current design depends on
it.

**`TransportStatusCode` replaces every free-text `reason: String`**
(`device_connection::transport`). `RowState` collapses from the original
three-way `Unconfigured`/`Configured`/`Live` to two cases, since "live" is
now the orthogonal `primary` flag rather than a row state:

```rust
enum RowState {
    Unconfigured { code: TransportStatusCode },
    Configured { code: Option<TransportStatusCode> },  // None = green
}

enum TransportStatusCode {
    NetworkUnavailable, BluetoothNotSupported, BluetoothDisabled,
    BluetoothNoAddress, BluetoothNotOsPaired,
    AwaitingFirstAck, PingMissed { count: u32 },
}
```

The frontend renders an "i" icon with a daisyUI tooltip next to any row
carrying a code, looking up display text in `utils/transportStatusCodes.ts`
— the only place English text is attached to a code. The `code` tag itself
(the backend's `#[serde(tag = "code", rename_all = "snake_case")]`
discriminant) is the stable key a future locale table keys translations
off of, without any backend change required to add a language.

### What Phase 1 still does, unchanged

WebSocket-native ping/pong (Phase 1, above) is untouched — it remains the
mechanism that detects a genuinely dead `Link` at the transport level for
Network (Bluetooth's own GATT disconnect callback still covers that job on
its side). The new app-level `Ping`/`Pong` is a different, higher layer:
it proves the *authenticated session* is exchanging traffic in both
directions, which a merely-alive `Link` doesn't by itself guarantee (e.g. a
link that's technically open but whose peer's `run_session` task has
wedged).

## Consequences

- Phase 1 adds no wire protocol at all — `Message::Ping`/`Pong` are
  WebSocket control frames handled entirely below `Link`, invisible to
  `PeerFrame`/`recv_frame`/an old peer's protocol version. An old Fini
  build on the other end of a `TcpWsLink` still responds to WS-level
  `Ping`s exactly the same (that's `tungstenite`'s job, unrelated to
  Fini's own protocol versioning), so this phase is safe against every
  peer, old or new, automatically.
- Phase 1 changes real behavior for every existing **Network** session
  (Bluetooth is already correct, unchanged): a currently-live TcpWs
  session that goes silent will now be torn down within ~45s instead of
  persisting indefinitely. This is the intended fix, not a side effect to
  guard against.
- **Superseded by the Revision above**: Phase 3's `SwitchTransport`
  negotiation, its exception to ADR-0001's sticky-handoff invariant, and
  Phase 2's dial-failure-counter reliability model (`tcp_dial_failure_count`
  / `bluetooth_dial_failure_count` / `*_effectively_reliable`/`_available`)
  no longer exist in the codebase — the invariant they carved a narrow
  exception into is itself retired, and the counters are replaced outright
  by the ping/ack proof. Historical reasoning for why Phase 3 originally
  needed that exception is left in place above for context, not as a
  description of current behavior.
- The new app-level `Ping`/`Pong` (the Revision) needs the same treatment
  `BluetoothAddressUpdate` got in ADR-0002 and `SwitchTransport` had:
  gated on `peer_protocol_version` learned during `Auth`/`AuthOk`, never
  sent proactively to a peer that hasn't proven it understands it. A peer
  on an older build simply never reaches green on any transport
  (`AwaitingFirstAck` forever) rather than having its session torn down —
  a degraded but correct outcome during a mixed-version rollout.
- Mixed-version pairs (one side upgraded, one still on the pre-revision
  sticky single-session model) degrade gracefully but not symmetrically:
  the old-build side still enforces "one session per peer" on its own
  accept path, so the pair gets at most one working transport until both
  sides upgrade — not a regression for a single point release with both
  ends controlled by the same user, not specifically engineered around
  further.
- No change to `specs/device-connect/README.md`'s trust model: pairing
  and disable/unpair semantics are untouched by this ADR. `Ping`/`Pong` is
  only ever exchanged on an already-authenticated session (post-`AuthOk`),
  same trust level as `SyncEvent`/`Ack`, not the pre-auth discovery-
  metadata tier `PairRequest`/`DiscoveryHello` sit at.
