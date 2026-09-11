# 0005 — An owned state machine for a peer's transport link

## Status

Accepted, not yet implemented. Supersedes the status model in ADR-0003's
revision, which this replaces rather than extends.

## Context

ADR-0003's revision introduced a four-state row model and, later, a two-case
`RowState`. Both describe how a transport row should *look*. Neither describes
how a link *behaves over time*, and the gap has been paid for repeatedly.

**There is no state machine today.** `build_transport_statuses` is a pure
projection: it takes twelve flat facts and derives a display shape. It has no
transitions, because it is not a machine. The actual state lives in four
independent stores that nothing reconciles:

| Store | Holds | Owner |
|---|---|---|
| `peer_sessions` | whether a session is claimed | `DeviceConnectionState` |
| `peer_transport_ack` | whether liveness is proven | `DeviceConnectionState` |
| `dial_exhausted`, `dial_backoff_until`, `in_flight_dials`, `accepting_side_unconnected_since` | whether an attempt is happening | process-global maps in `transport/ble.rs` |
| `paired_devices` columns | preconditions | SQLite |

Because no code owns a transition, no transition can have an effect. That is
the root cause of the bug this ADR starts from, not a detail of it.

**The failure it produced.** On a Pixel 6 Pro paired with a Linux desktop:

```
04:59  session established, auth OK, row green
       ...peer stops answering...
       3 missed ping cycles (~45s) -> row amber, code PingMissed
06:43  1h44m later: the session is still claimed
       [gate] Bluetooth auth from 1a3ae309-... rejected: session already active
```

The desktop refused every incoming connection from that peer for over an hour
in favour of a link that had been dead the whole time, while the peer's own
dial side wound down to `bluetooth_dial_exhausted`. `PingMissed` changed a
colour and nothing else: there is no edge out of it, because edges do not
exist.

**Four further defects of the same origin:**

1. *Two authorities that can disagree.* `RowState::Configured` needs a claimed
   session; its code comes from the ack table. A claimed-but-dead session
   yields "Configured + PingMissed" indefinitely. `try_claim_session` consults
   only the first, so it rejects a live peer in favour of a corpse.
2. *One enum, four natures.* `TransportStatusCode` mixes platform capability
   (`BluetoothNotSupported`), configuration (`BluetoothDisabled`,
   `BluetoothNoAddress`, `BluetoothNotOsPaired`), attempt lifecycle
   (`Connecting`, `BluetoothDialExhausted`) and proof lifecycle
   (`AwaitingFirstAck`, `PingMissed`). Different owners, different update
   paths, different exit conditions — but one flat type, so nothing forces
   exhaustive reasoning about which transitions are legal.
3. *Half the machine lives outside the type.* "Am I trying to connect" is
   spread across four process-global maps. The comments on those maps record a
   series of P1 fixes for exactly the failure mode that spread invites: stale
   flags, rows stuck on "Still connecting…".
4. *No notion of a relationship.* Observed: desktop `ping_missed`, phone
   `bluetooth_dial_exhausted` — two contradictory descriptions of one link.
   ADR-0003's revision was itself motivated by "two devices showed different
   colours"; this is the same class recurring, because the fix addressed the
   projection rather than the model.

## Decision

Introduce an explicit state machine per `(peer, transport)` that **owns** the
state. Events cause transitions, transitions have effects, and the UI row
becomes a projection of one value instead of twelve scattered facts.

### States

```
Unavailable { reason }   preconditions unmet (platform, settings, OS bond, no network)
Idle { retry_at }        eligible, nothing in flight; retry_at carries backoff
Dialing { since }        outbound attempt in flight
Authenticating { since } link up, auth exchange running
Proving { since }        session claimed, first ping/ack round not yet complete
Live                     liveness proven, currently
Fading { since }         proof lapsed; grace running toward teardown
GaveUp { since }         automatic retries exhausted; user action or an inbound session leaves it
```

### The invariant that makes lying impossible

> A session exists **if and only if** the state is one of
> `Authenticating`, `Proving`, `Live`, `Fading`.

Every "the row says connected but nothing is" bug in this codebase's history is
a violation of that sentence. Making it an invariant of a single owned value,
rather than an emergent property of four stores, is the point of this ADR.

### Transitions that carry effects

| From | Event | To | Effect |
|---|---|---|---|
| `Live` | proof lapsed | `Fading` | start grace timer |
| `Fading` | proof complete | `Live` | cancel grace |
| `Fading` | grace expired | `Idle` | **close the link, release the slot** |
| any session state | session ended | `Idle` | release the slot |
| `Idle` | eligible and this side is the designated dialer | `Dialing` | start dial |
| `Dialing` | no auth within `AUTO_RETRY_WINDOW` | `GaveUp` | stop automatic retries |
| `GaveUp` | user retry, or inbound session offered | `Idle` / `Authenticating` | resume |
| any | preconditions lost | `Unavailable` | tear down anything live |

`Fading -> Idle` is the edge that does not exist today, and its absence is the
whole bug.

### Decisions taken, with reasoning

| Question | Choice | Why |
|---|---|---|
| How does a pair agree? | Honest local state; no protocol change | The divergence was not two peers miscommunicating — one held state that did not match reality. Exchanging views about a wrong state does not make it right. With teardown on proof loss, both sides converge on their own: the desktop drops the corpse, the peer's dial succeeds. |
| Machine's role | Single source of truth | The scattered maps fold into it; the UI row becomes a computed projection. The only option under which the stores cannot disagree again. |
| Model both ping directions? | One flag in the state | Both directions stay tracked internally as an implementation detail of the transition, but the state exposes "proven / not proven". Keeps the type small; direction detail stays available in logs. |
| When to tear down? | Grace period after amber | Proof already tolerates ~45s of silence (3 × 15s). Grace adds ~30s, so a link is torn down after ~75s of genuine silence — long enough to ride out a radio glitch without reconnect churn, short enough that nothing hangs for an hour. |
| Rollout | One piece, in PR #168 | User's call. Noted under Risks. |

### Projection to the UI

| State | Row |
|---|---|
| `Unavailable` | gray, with reason |
| `Idle`, `Dialing` | amber, "connecting" |
| `GaveUp` | gray, clickable to retry |
| `Authenticating`, `Proving` | amber |
| `Live` | green |
| `Fading` | amber |

`primary` stays orthogonal, as ADR-0003's revision established.

### Reactive delivery

`device-connection://session-changed` already exists but carries only
`established: bool`, and the frontend answers it by re-polling — a
cache-invalidation signal, not state. It becomes the state itself, so the
frontend's computed property derives directly from one pushed value instead of
round-tripping. Polling stays as a self-heal for a missed event, as today.

## File changes

**New** — `src-tauri/src/services/device_connection/link_state.rs`: the state
enum, the event enum, and a pure `transition(state, event) -> (state, Vec<Effect>)`
function. Pure so the whole table is unit-testable without a radio, a peer, or
a DB.

**Modified**

- `device_connection/mod.rs` — holds one machine per `(peer, transport)`;
  `try_claim_session` / `release_session` / `note_ping_*` become event
  submissions rather than direct mutations.
- `device_connection/transport.rs` — `build_transport_statuses` shrinks to a
  projection of the machine's state. `TransportStatusCode` splits: precondition
  reasons stay, attempt/proof codes are derived from the state instead.
- `transport/ble.rs` — the four process-global maps are deleted; their content
  becomes `Idle { retry_at }` and `GaveUp`. `should_dial_peer` becomes an
  effect of entering `Idle`.
- `space_sync/session.rs` — the gate consults the machine; the ping handlers
  submit events.
- `src/stores/device.ts` — the row becomes a computed over the pushed state.

## Consequences

The pure transition function makes the machine testable exhaustively, which the
current design cannot be: today's behaviour can only be observed by assembling
a peer, a radio and a DB, which is precisely why these defects were found on
hardware rather than in CI.

`TransportStatusCode` becomes a smaller, honest type: preconditions only.
Everything else is read off the state.

The change is large and touches the connection path on both transports. This is
the third attempt at this model (ADR-0003, its revision, now this), and the
first that addresses ownership rather than presentation.

## Risks

Landing in one piece, per the rollout decision, means there is no intermediate
version to verify on hardware — and hardware verification is what found every
defect in this area, including all four listed above. Mitigation: the pure
transition function is exhaustively unit-tested before anything is wired up, so
only the wiring is unverified when it first reaches a device.

Deleting the ble.rs maps removes code whose comments document several past P1
fixes. Each of those behaviours needs a corresponding transition or it
regresses; they are enumerated in those comments and must be walked one by one.

## Verification

- Unit: the transition table, exhaustively — every (state, event) pair,
  including the ones that must be no-ops.
- Unit: the invariant — a session exists iff the state is one of the four
  session states — asserted after every transition in a property test.
- Hardware, the case this ADR starts from: establish a Bluetooth session, kill
  the peer's radio, confirm the row goes amber at ~45s, the link is torn down at
  ~75s, and a subsequent inbound connection from that peer is *accepted* rather
  than refused.
- Hardware: both devices reach green and neither reports a state the other
  contradicts.
- `make e2e-devices` stays at 9/9 on the external-actor pair.

## Prerequisite this ADR does not supply: the bond must be an LE bond

Everything above is downstream of something that has to exist first, and
discovering how strictly it has to exist cost a full day of hardware
debugging. Recorded here so the next person does not repeat it.

`bluetooth_dial_candidates` dials a peer's **stored identity address**, and
`check_bluetooth_bond` requires that address to be OS-bonded right now. Both
are necessary, and neither is sufficient, because the *kind* of bond decides
which transport BlueZ then uses:

- **No bond at all.** Android advertises with a resolvable private address.
  Without a bond BlueZ cannot resolve it to the identity address, so the dial
  has no reachable target. `bluetoothctl pair <identity>` cannot help either --
  it answers `Device not available`, because the device was never discovered
  under that address.
- **A DUAL bond**, which is what pairing through Android's own Settings
  produces against a desktop, because the desktop also advertises classic
  profiles. BlueZ's `Device1.Connect()` then prefers BR/EDR and the dial fails
  with `br-connection-unknown` / `br-connection-canceled`. There is no
  transport argument on `Connect()` to override this -- BlueZ keeps one
  `Device1` per remote identity and merges both roles onto it.
- **An LE bond** is the configuration in which the dial works. The pair that
  reached green on hardware had exactly that.

So a working BLE link needs an LE bond, and nothing in the shipped product
creates one: the in-app Bluetooth pairing flow is not delivered yet. Pairing
through Android Settings actively produces the wrong kind.

**Consequence for planning.** The peripheral-session defect this ADR's
follow-up targets, and the state machine above, are both downstream of that
missing step. A user cannot reach them today, because the link they would
exercise cannot be established in the first place. Delivering BLE pairing is
the prerequisite, not a parallel track.

**Consequence for method.** Two further traps, both of which produced
confident and wrong readings before being spotted:

- Two desktop app instances ran simultaneously for hours, because the
  `make desktop-debug` guard only protects that target and the binary was
  launched directly. Two instances contend for one adapter's scanning and
  advertising, and quietly distort everything measured through it.
- The host adapter itself became unstable under that load, to the point of
  dropping the machine's other Bluetooth devices. Findings taken from an
  adapter in that state -- including the `br-connection-*` failures and the 3s
  peripheral-session lifetime -- are worth re-confirming on a healthy one
  before being built on.

## Open questions

- Grace duration is set at 2 ping cycles (~30s) by the reasoning above, but has
  not been validated against a real flaky link. Worth tuning once there is
  hardware evidence of reconnect churn or its absence.
- `GaveUp` currently has a 60s `AUTO_RETRY_WINDOW`. Whether that survives
  unchanged under the new model, or becomes a backoff ladder in `Idle`, is not
  settled.
- Whether the Network transport needs `GaveUp` at all, or only Bluetooth does.
