# 0002 — Bluetooth address auto-exchange, live transport status, and BLE-first pairing

## Status

Proposed. Three phases, in dependency order:

1. **MAC auto-exchange** for peers already paired over network.
2. **Live transport status** — `device_connection_transport_statuses` reports
   whether a session is actually established right now, not just whether the
   precondition for one is met.
3. **BLE-first pairing** — discover and pair with a device that has never
   been paired at all, entirely over Bluetooth, mirroring the existing
   network pairing UX. Depends on an upstream `ble-gatt` change (advertised
   manufacturer/service data), sequenced as its own PR before this phase's
   Fini-side work begins.

## Context

`transport::ble` (this repo's Android port, and the Linux BlueZ adapter
before it) requires each side to already know the peer's Bluetooth MAC
address before it can dial. Today that address is entered by hand on
`DeviceView.vue`, for a device that is *already* paired over network. Using
the feature in practice surfaced two problems:

1. **Manual entry is the wrong default when the devices already trust each
   other.** Two network-paired devices have an authenticated channel
   already; asking the user to copy a MAC address between them by hand is an
   avoidable step, not a security requirement — `specs/device-connect/
   README.md` already establishes that Fini app pairing, not OS Bluetooth
   pairing, is the trust boundary.
2. **The Bluetooth status row's "available" flag doesn't mean what a user
   expects.** It reports a static precondition
   (`bluetooth_enabled && has_address && os_bonded`, re-evaluated on every
   `space_sync_tick`), not "this transport actually has a live session
   right now." A user reasonably reads a green status as "if I make a
   change, it'll sync immediately" — which the current flag cannot promise.

Separately, all pairing today assumes a network path exists between the two
devices during the pairing handshake itself (mDNS discovery + a one-shot
`PairRequest`/`PairAccept`/`PairComplete` exchange over a WebSocket). Two
devices with no network path in common — different Wi-Fi networks, one with
Wi-Fi off entirely — cannot pair at all today, even though the whole point
of adding Bluetooth as a second transport was to cover exactly that case.

### What already exists and is reused, unchanged

- `session::run_peer_gate` is transport-neutral and already dispatches
  `PairRequest`/`PairAccept`/`PairComplete` as its first three frame types,
  before any `Auth` frame — see ADR-0001. Nothing about the pairing protocol
  itself needs to change for it to run over a fresh, unauthenticated BLE
  `Link` instead of a WebSocket one.
- `DeviceConnectionState` already exposes `subscribe_lifecycle()` (a
  broadcast of session-established/ended events) and `session_kind()` —
  ADR-0001 built these but deliberately left `device_connection_transport_
  statuses` reporting the static heuristic instead of live session state
  ("follow-up work, not required for this PR's CI/protocol scope"). Phase 2
  is that follow-up.
- `ble_gatt::Backend::scan()` and the reserved `Candidate` type already
  exist at the library level (ADR-0001: "the real Bluetooth adapter (PR B)
  needs genuine discovery ... and will use the reserved `Candidate` type for
  it"). Nothing has consumed them yet.
- `bluetooth_dial_candidates`/`is_still_bluetooth_eligible`/
  `check_bluetooth_bond` (this session) already establish the pattern of
  treating a stored Bluetooth address as untrusted metadata until the real
  Fini `Auth` handshake — over that same address — confirms it.

### Platform constraint that shapes Phase 1

Android apps cannot read their own device's real Bluetooth MAC address via
the public API — `BluetoothAdapter.getAddress()` has returned a dummy
`"02:00:00:00:00:00"` since Android 6.0, with no supported workaround for a
normal app. Linux (`bluetoothctl show`) has no equivalent restriction. This
is asymmetric and permanent (a platform privacy protection, not a bug), and
it rules out a single symmetric "exchange my own address" mechanism.

## Decision

### Phase 1 — MAC auto-exchange for already-paired devices

Two complementary mechanisms, matched to which side can know its own
address:

- **Self-report (Linux → peer).** A new `PeerFrame::BluetoothAddressUpdate
  { address: String }`, sent once per authenticated network session
  (immediately after `AuthOk`, on whichever side can read its own real
  adapter address — today, Linux only) and again if the local address
  changes mid-session. The receiving side's `run_session` handler persists
  it as that peer's `bluetooth_address` if not already set. This is safe to
  trust immediately: it arrives over an already-authenticated `PeerFrame`
  channel, the same trust boundary every other post-auth frame uses.
- **Discover (peer → Android, or peer → Linux/Android generally).** A
  per-peer, user-triggered, time-boxed action — a "Find via Bluetooth"
  control on that peer's row/detail in `DeviceView.vue` — that:
  1. Requests Bluetooth runtime permissions if not already granted (same
     click-triggered pattern as the existing "Enable Bluetooth" toggle —
     see `BluetoothPairing.requestPermissionsIfNeeded`'s doc comment from
     this session; a scan button is exactly the same class of genuine user
     action, not app-startup).
  2. Scans for nearby `FINI_BLE_SERVICE_UUID` advertisers for up to 60
     seconds (button shows a scanning state, disabled for the duration).
  3. For each discovered MAC, opportunistically connects and sends the
     normal `PeerFrame::Auth` with the *already-known* `peer_device_id`
     (from the existing network pairing) — no new protocol needed here. A
     real `AuthOk` is the proof this MAC belongs to the expected peer, not
     some other nearby Fini install.
  4. On the first `AuthOk`, stops scanning, persists the address, and —
     see below — enables Bluetooth for the pair immediately.

**Auto-enable on success, not auto-populate-then-wait.** Both mechanisms
enable Bluetooth for the pair as soon as the address is confirmed, rather
than only filling the field and leaving `bluetooth_enabled` for a separate
manual step. This is a deliberate exception to the existing
"Bluetooth requires explicit per-pair enablement" rule in `specs/
device-connect/README.md`: that rule exists to stop an address being
*trusted* without confirmation, and both paths above already require
confirmation — self-report arrives over an authenticated channel, and
discovery requires a live `AuthOk` from the discovered address itself. The
explicit-action requirement is satisfied by the click that started the
process (the scan button), not by a second click afterward.

**Manual entry is kept** as a fallback on `DeviceView.vue` for when scanning
fails (peer out of range, their Bluetooth off, transient OS scan issues) —
this feature must never regress below what exists today.

### Phase 2 — Live transport status

Add a `connected: bool` field to `TransportStatus` (`device_connection::
transport`), computed from `DeviceConnectionState::session_kind(peer_id) ==
Some(kind)` for that row — i.e., is there an authenticated session live on
*this specific* transport right now. This is a new field, not a
redefinition of the existing `available`/`preferred` fields: those keep
their current "precondition met" meaning (still useful — "Bluetooth is
configured and bonded" is worth showing even when the live session happens
to be running over network instead, since network is preferred whenever
both are available per ADR-0001's selection order).

No new background work is needed: `session_kind()` already exists and is
already updated by `try_claim_session`/`release_session`. `DeviceView.vue`
renders `connected` as the primary color/status signal per row; `available`
becomes secondary/explanatory text ("configured, not currently connected"
vs. today's undifferentiated green).

This deliberately does **not** add a unified any-transport "peer online"
indicator — the per-transport rows stay per-transport, matching the answer
given during design review.

### Phase 3 — BLE-first pairing

**Prerequisite, separate PR against `ble-gatt`:** extend `GattServiceSpec`
(currently `{ uuid, characteristics }` only) with optional
`manufacturer_data: BTreeMap<u16, Vec<u8>>` and/or `service_data:
BTreeMap<ServiceUuid, Vec<u8>>`, mirroring what `DiscoveredPeer` already
carries on the scan side, and thread it through `Backend::advertise()` for
both the Linux BlueZ backend and the Android `BleGattBridge`/
`startAdvertising`. This is a symmetric, additive change to an existing
API, not a new concept in that library. Fini's `Cargo.toml` git rev is
re-pinned to the merged commit before Phase 3 work here begins.

**Unified add-mode.** `device_connection_enter_add_mode`/`leave_add_mode`
gain a second effect: alongside the existing mDNS beacon, the local
Bluetooth peripheral's advertisement gets a 1-byte add-mode flag set in
manufacturer data (comfortably inside the legacy 31-byte advertisement
budget, which the existing 18-byte service UUID + 3-byte flags already
mostly consumes). One toggle, both transports — matches "I want to pair
with something nearby" as a single mental action.

**Discovery is opt-in-filtered, not blanket-probing.** The sender's device
scans, but only ever connects to a MAC whose advertisement already carries
the add-mode flag — devices not in add-mode are invisible and never
touched. This was a deliberate design requirement: connect-and-probe-
everyone-nearby (the initial proposal) was rejected specifically because it
would touch every nearby Fini install regardless of whether they'd opted
into being found.

**New pre-auth "hello" exchange for flagged candidates only.** For each
add-mode-flagged MAC, the sender connects and sends a new
`PeerFrame::DiscoveryHello` (pre-auth, alongside `PairRequest`/`PairAccept`/
`PairComplete` in `run_peer_gate`'s first-frame dispatch); the receiver
replies with `DiscoveryHelloReply { device_id, hostname }`, mirroring the
fields `DiscoveryBeacon` already carries for mDNS. This is what a BLE
advertisement's ~13 remaining bytes (after the service UUID and add-mode
flag) cannot carry directly.

**Unified candidate picker.** `AddDeviceView.vue`'s existing candidate list
gains Bluetooth-discovered entries alongside network-discovered ones, each
tagged with which transport found it. Bluetooth scanning runs for as long
as `AddDeviceView` is open and the local device is in add-mode — this view
is specifically the pairing view, so continuous scanning here doesn't
conflict with the "don't scan on unrelated pages" principle Phase 1's scan
button was built to satisfy elsewhere.

**Pairing itself is unchanged.** Once a candidate is picked, the existing
`PairRequest`/`PairAccept`/`PairComplete` flow runs exactly as it does over
network today, just over the BLE `Link` the discovery step already
established. `run_peer_gate` needed no changes for this — ADR-0001's
transport-neutral design is what makes this phase mostly new discovery
plumbing, not a new pairing protocol.

**`PairCompletePayload` exchanges both transports' details when
available**, regardless of which transport carried the pairing itself —
network endpoint and Bluetooth address both get sent if the sending side
has them. A pair completed over Bluetooth alone gets a network endpoint
filled in for free if both devices happen to share a network, and
vice versa; Phase 1's discovery button becomes the fallback for pairs
missing one side's detail (paired before this feature existed, or one side
didn't have Bluetooth ready at pairing time), not the primary mechanism for
new pairs.

**On a Bluetooth-carried `PairComplete`:** the new `paired_devices` row is
created with `bluetooth_enabled = true` and `bluetooth_address` populated
immediately (they just proved reachability by completing the pairing
handshake itself) — consistent with Phase 1's "confirmation already
happened, don't ask twice" reasoning.

## Consequences

- Phase 1 and 2 touch no wire-incompatible surface: `BluetoothAddressUpdate`
  is a new, additive `PeerFrame` variant (old clients simply don't send or
  understand it — no breaking change, unlike ADR-0001's frame envelope
  migration). Phase 2 is a new response field, additive on the existing
  `device_connection_transport_statuses` command.
- Phase 3 is **not** independently shippable before its `ble-gatt`
  prerequisite lands — Fini-side work on the add-mode advertisement flag
  cannot start until `GattServiceSpec` supports manufacturer/service data
  upstream and the git rev is re-pinned. Phases 1 and 2 have no such
  dependency and can ship first.
- `DiscoveryHello`/`DiscoveryHelloReply` are new pre-auth, unauthenticated
  frame types — same trust level as the existing pre-auth `PairRequest`
  (untrusted discovery metadata, per `specs/device-connect/README.md`),
  not a new category of exposure.
- The BLE-first pairing flow means every nearby add-mode-flagged Fini
  device gets a connect-and-hello from anyone scanning nearby while in
  add-mode — bounded, expected exposure while a user has deliberately
  opted into being discoverable, comparable to today's mDNS beacon
  broadcast during add-mode.
- Phase 3's advertised add-mode flag toggling means the peripheral's
  advertisement payload changes twice per add-mode session (flag on, then
  off) — implemented as a stop+restart of the existing always-on
  advertisement, not a second concurrent advertisement.
