# 0008 — A channel is a thing the user chose

## Status

Accepted, implemented. Design: the `device-pairing` kit in the Fini Design
System project (`ui_kits/device-pairing/`), drawn from the grilling captured
in `fini-wiki/raw/2026-09-17-device-pairing-redesign-brief.md`.

Inline renaming is designed but not built here — it stays with issue #117.
Auto-selecting a channel is deferred to #176.

**Not yet exercised on hardware.** Everything below is verified by unit
tests, the actors e2e lane and a production build. The Bluetooth half in
particular — an adapter switched off at the OS level, a peer walking out of
range — has only been reasoned about and tested against fakes.

## Context

Two devices connect over one of two channels: the local network, or
Bluetooth. Since ADR-0003 both can be connected at once, with one of them
*primary*; since ADR-0005 each proves its own liveness continuously; since
ADR-0006 Bluetooth needs no OS bond.

None of that reached the person using it. The Device page had accumulated
five controls for one idea — "Find via Bluetooth", a text field for a MAC
address, an Enable/Disable button pair, a star for the preferred transport,
and a "Retry dial" button — and the Add Device page decided the channel by
itself, captioning whatever it found "via network" or "via Bluetooth" after
the fact.

The result was a page that could not answer the only three questions anyone
brings to it:

- **Is it connected?**
- **Has my stuff synced?**
- **If not, why not?**

The third was the worst. A row would say `Unavailable`, or sit on
"Still connecting…" indefinitely, or — after ADR-0006 — report that a peer
"isn't nearby" when the truth was that this machine's own Bluetooth was
switched off. Every one of those is a status code wearing a sentence.

## Decision

**The channel is chosen, not inferred.** Adding a device asks which channel
first, before any discovery runs. This costs the user a decision they did
not previously have to make, and buys back an outcome they can predict; the
automatic version is a separate feature (#176) layered on top, not the
default that hides this one.

**Trust is per pair, not per channel.** Adding a second channel to a device
that is already paired needs no six-digit code — the devices exchange what
they need over the channel they already trust, and the second person is
never interrupted. Pairing and channel setup are the same modal with the
code step switched off, so the difference is visible rather than implied.

**Pairing is a modal.** It is a short, two-person ceremony with a beginning
and an end: one person acts while the other waits, and both screens have to
keep saying whose turn it is. A page cannot do that — it looks like a form,
and a form does not explain why nothing is happening. `AddDeviceView` is
deleted; `/settings/add-device` redirects to Settings.

**Every row says why, by name.** `transportStatusCodes.ts` remains the only
place English is attached to a code, and every string in it now names the
device and says which machine is at fault. "Pixel 8 isn't nearby" and
"Bluetooth is off on this computer" are different sentences about different
problems, and the person can only act on one of them.

**Both channels get the same switch.** Network gains `network_enabled`, the
counterpart to `bluetooth_enabled` it has never had. The switch does three
things — filters the peer out of `tcp_ws::spawn_dial_loop`, closes the open
session, and releases a Network pin — because a switch that greys a row
while traffic keeps flowing is a lie.

## "On, waiting", and why it is a state rather than a message

Switching a channel on when the thing it needs is missing used to fail: an
error, and the switch snapped back. That is the wrong shape for the most
common case, which is a laptop with Bluetooth turned off at the OS level.
Nothing is broken and nothing needs deciding — the user's intent is
perfectly clear and simply cannot be acted on yet.

So it does not fail. The switch stays on, the channel starts by itself when
the radio returns, and the row reports `BluetoothAdapterOff` in the
meantime: knob in the on position, track gray rather than green.

The design put that explanation in a toast fired at the moment of the flip.
It is in the row's own reason line instead, and the reason it moved is that
this is a state the person *sits in* rather than passes through — a toast is
gone in four seconds, and the question it answers survives it.

Getting that reason to appear promptly needed one new signal.
`is_bluetooth_adapter_unavailable` is *observed*, not polled: the dial loop
already scans every `SCAN_PERIOD_*`, so recording whether those attempts
were accepted keeps it fresh for free, and "we asked the radio to do
something and it refused" catches an adapter that reports itself powered and
then declines to scan. Only a real use records health — `backend()` caches
its handle in a `OnceCell`, so a cached success proves nothing about
hardware that was switched off afterwards.

That is honest but, since ADR-0007 moved the tick to 30s, unhurried. It is
invisible in the background and wrong in the one second the person is
watching the switch they just flipped, so **the flip pays for a direct probe
and nothing else does**: `device_connection_probe_bluetooth_adapter` opens a
discovery session, drops it, and records the result.

The ordering in `bluetooth_unconfigured_code` is load-bearing.
`BluetoothAdapterOff` sits above `BluetoothPeerNotNearby`, because with our
own radio off we have not looked for the peer at all — reporting "isn't
nearby" would be a claim we have no evidence for, about the wrong device.
It sits below `BluetoothDisabled`, because a channel the user switched off
has no business complaining about hardware.

## The sync queue

"Has my stuff synced?" had no answer at all; `pending_event_count` existed
but was rendered as `pending 3 · outbox 10 · acked 10` next to the space
mappings.

It is now its own section: one honest line — "Everything synced", with when
the last change reached the device, or "N changes waiting" — expanding to
the titles of the quests actually waiting. `space_sync_queue_summary` reuses
`load_unacked_events_for_peer`, the same query the sender uses, so the
number on the page cannot drift from what the next tick would push.

Titles resolve for Quests and Spaces only. Reminders, checklist activity and
focus history are bookkeeping the person never named, so they are counted
but not listed — inventing "Checklist activity" as a label would fill the
list with words that mean nothing to the reader.

## Consequences

**Deleted, deliberately:** the MAC address field (people do not have MAC
addresses, and since ADR-0006 the stored address is not what a dial uses),
the Enable/Disable button pair, the standalone Retry button, and
`AddDeviceView` itself.

**Not built, deliberately:** the prototype gives each channel row a trash
icon alongside its switch, enabled only while the channel is off. With
`network_enabled`/`bluetooth_enabled` there are two states, on and off, so a
delete distinct from "off" would either be a no-op or would need a third
state invented to justify it. The switch is the whole control.

**Migration 23 defaults `network_enabled` to 1**, unlike migration 19's
`bluetooth_enabled`. Bluetooth is opt-in; Network is what every existing
pair is already syncing over, and defaulting it off would disconnect every
pair in the field on upgrade.

**The e2e pairing helper changed shape.** `pairActorsViaUi` now opens the
modal from Settings and picks a channel; acceptance moved inside the dialog,
so it no longer keys off the sender's hostname.

## Verification

Done:

- `cargo test --features ui-plane --lib` — 243 passed, including a migration
  test that winds a database back to the pre-23 shape with a pair stored in
  it and asserts Network survives the upgrade, and row-state tests pinning
  both new orderings.
- `npm run build` (`vue-tsc` + `vite build`) clean; frontend unit tests pass,
  including a rewritten `DeviceView.spec.ts` and a new
  `PairDeviceDialog.spec.ts`.
- `make e2e-ci` — main lane 36 passed / 2 failed, sim 2/2, BLE 1/1.

The four new actors tests in `device-page.spec.ts` are the ones worth
naming, because they cover what unit tests structurally cannot — the store
is mocked there, so it proves the page renders what it is handed, not that
two real apps behave that way:

- turning a channel off closes the session, asserted on
  `device_connection_session_transport` rather than on the row's colour;
- a channel switched on with no usable radio reaches "On, waiting" and
  blames *this* device, explicitly not the peer;
- the sync queue reports a backlog while nothing can reach the peer, then
  drains when the channel returns;
- unlinking names the spaces that stop and promises nothing is deleted.

The "On, waiting" test runs in the plain `actors` lane rather than
`actors-ble`: the e2e container has a D-Bus system bus with no `bluetoothd`
on it, so `LinuxBackend::new()` genuinely fails there — the same condition
as Bluetooth switched off on a laptop, arrived at honestly. The BLE lane
could not host it anyway, since `ble-gatt`'s mock fault injection is
`Local`-only and panics across the broker.

The 2 remaining failures are `reminder-notification-actions`, and they are
**pre-existing**: a run of clean `HEAD` in a throwaway worktree failed 3 / 31
in the same lane. Their cause is an app-level `SIGABRT` under the e2e
container, unrelated to devices, tracked separately.

## Verified on hardware

Pixel 6 Pro over USB against a desktop debug build, both on 10.0.0.0/24,
with a pair that predates this work (2026-09-13) and real Spaces.

- Migration 23 upgraded that existing database in place: `network_enabled`
  came back `true`, so the pair kept syncing over the channel it was formed
  on — the asymmetry with `bluetooth_enabled` doing its job.
- Both channels connected **at once**: `network:configured PRIMARY`,
  `bluetooth:configured`, rendered as two live rows with the star on Network.
  Bluetooth authenticated over a real radio (`via 52:4D:3E:99:BC:55`).
- A quest created on the desktop reached the phone and was acknowledged in
  **6.0s**, against a 30s backstop — the event-driven path working on the
  platform that had none before the review fixes above.
- Reasons named the right machine. With the desktop's radio scanning
  normally the Bluetooth row said "fini-75700b2e isn't nearby", not
  "Bluetooth is off on this computer" — the ordering in
  `bluetooth_unconfigured_code` behaving correctly unprompted.
- The sync queue, stalled, read "2 changes waiting · Nothing can reach
  fini-75700b2e until a channel connects" and listed both real quest titles;
  drained, "Everything synced · Last change reached fini-75700b2e just now".

**Two defects this found that the whole test suite did not.** Both are
fixed above, and both were in code written for this change:

1. Work emitted while a peer was unreachable stayed in the outbox after the
   session returned. The keeper had woken once, found nothing sendable and
   gone back to waiting, and desktop has no periodic arm to catch it. A
   forced tick sent both events instantly, which is what proved the send
   path was fine and only the trigger missing. `claim_session` now raises
   the signal.
2. **The Network switch did not stop traffic.** The row read "Off" over a
   live `tcp_ws` session, because only the *outgoing* dial was gated — the
   peer kept dialling in and `check_paired` let it through. Exactly the lie
   this design exists to remove, and exactly the failure
   `check_bluetooth_enabled` had already been written to prevent for the
   other channel. `check_network_enabled` is its counterpart.

The second is worth dwelling on: `device-page.spec.ts` asserted that
behaviour and **passed**, because it sampled once three seconds after the
toggle and the peer had not re-dialled yet. It now samples repeatedly with
ticks in between. A single click on a real pair exposed in seconds what a
green suite could not.

Owed:

- Switching Bluetooth on with the laptop's own radio off, to see "On,
  waiting" on hardware. Needs the host radio disabled, which would drop any
  Bluetooth peripherals in use, so it is left for a deliberate session.
- A first-time pairing run through the new modal between two devices that
  have never met; the hardware pair here already existed.
- The pairing modal's `declined` step is unreachable in production —
  declining is never sent to the requester (issue #177). Its rendering is
  unit-tested; it has no e2e, deliberately, because an e2e would have to
  fabricate the state it asserts.
