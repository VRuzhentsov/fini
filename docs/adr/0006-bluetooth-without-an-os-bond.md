# 0006 — Bluetooth without an OS bond

## Status

Accepted, not yet implemented.

Supersedes the "OS Bluetooth pairing is a transport precondition" line in
`specs/device-connect/README.md`, which this removes rather than weakens.

## Context

A full day of hardware debugging established that fini's Bluetooth transport
cannot connect at all unless the two devices hold an OS-level bond **of the
right kind**, and that nothing in the product creates one.

The chain, each step evidenced on a Linux desktop paired with a Pixel 6 Pro:

1. `bluetooth_dial_candidates` dials a peer's **stored identity address**
   (`paired_devices.bluetooth_address`).
2. Android does not advertise with that address. It advertises with a rotating
   resolvable private address. Our own scanner saw the phone under
   `6C:29:7B:FE:9E:0E` and later `68:F0:23:21:44:2E` — both with the top two
   address bits `01`, the RPA marker — while the phone's identity address,
   `58:24:29:D8:0B:72`, never appeared in the air at all.
3. Resolving an RPA to an identity address requires the IRK exchanged during
   **bonding**. Without a bond, BlueZ cannot connect the advertisement to the
   stored device, so the dial has no reachable target.
4. Bonding through Android's own Settings against a desktop produces a **DUAL**
   bond, because the desktop also advertises classic profiles. `Device1.Connect()`
   then prefers BR/EDR and the dial fails with `br-connection-unknown` /
   `br-connection-canceled`. BlueZ exposes no transport argument to override
   this: it keeps one `Device1` per remote identity and merges both roles onto
   it.
5. An **LE** bond is the only configuration in which the dial works. Nothing
   shipped creates one — in-app Bluetooth pairing is not delivered — and the
   route a user would naturally take produces the wrong kind.

So the transport depends on a precondition the product cannot establish, and
which the user can silently destroy by re-pairing their devices the obvious way.

**What the bond actually buys, measured against what we assumed it bought:**

| Assumed | Actual |
|---|---|
| Trust in the peer | No. `specs/device-connect/README.md` already states the opposite: "OS Bluetooth pairing is a transport precondition only; Fini app pairing remains the trust boundary", and "Bluetooth discovery/connection metadata is untrusted until the existing Fini pair-auth session succeeds" |
| Channel confidentiality | No. Our GATT characteristics are declared with plain `PERMISSION_READ`/`PERMISSION_WRITE`, so nothing requires an encrypted link. A bond exists and protects nothing |
| Address resolution | **Yes — and only this** |

The bond, then, serves an implementation choice (dial a remembered MAC) rather
than a requirement. The requirement is "connect to the paired peer over
Bluetooth", which says nothing about how the peer is addressed.

## Decision

**Stop depending on the OS bond. Connect to whoever advertises our service, and
identify them with the app-level auth we already trust.**

1. **Dial by advertisement, not by address.** Scan for our service UUID —
   unique to Fini — and connect to what answers. The peer's link-layer address
   becomes an implementation detail we never store, compare, or depend on.
2. **Identify at the app layer.** The `Auth` frame already carries `device_id`,
   and `run_peer_gate` already rejects a peer that is not the expected one.
   This is unchanged: it is already the declared trust boundary, and it already
   works.
3. **Drop the bond checks.** `check_bluetooth_bond` and the OS-pairing arm of
   `bluetooth_dial_candidates` go away, along with `bluetooth_address` as a
   dial input. Whether the address is kept as diagnostic metadata is an open
   question below.
4. **Protect the channel above the transport, not at the link.**
   `secure_channel.rs` — today a stub sitting between the codec and the `Link`
   — becomes the place confidentiality lives, using an established protocol
   rather than anything hand-rolled.

### Why not link-layer encryption instead

Android can give us encryption for free: declaring characteristics with
`PERMISSION_READ_ENCRYPTED`/`PERMISSION_WRITE_ENCRYPTED` makes the stack demand
an encrypted link and bond if necessary, and the controller then protects the
whole session with no crypto code of ours. That is the standard BLE answer and
it is genuinely attractive.

It is the wrong answer *here* for two reasons:

- **It requires the bond**, so every failure above comes back: DUAL versus LE,
  rotating addresses, OS pairing prompts we do not control.
- **It covers one transport of two.** Our peer protocol is deliberately
  transport-neutral (ADR-0001), and the network transport is plaintext `ws://`
  today. Link encryption would protect the secondary transport and leave the
  primary one open. Protection belongs where the neutrality is.

A library-level option for this is being added to ble-gatt for other consumers;
fini is expected to leave it off.

## Consequences

**The circular dependency disappears.** Bluetooth stops needing a bond, so it
stops needing the in-app pairing flow that would create one, so it stops being
blocked behind undelivered work.

**Android's address rotation stops mattering**, as does the distinction between
LE and DUAL bonds, and with it a whole class of failure that is invisible from
inside the app and unfixable by the user.

**`specs/device-connect/README.md` line 29 must change.** "OS Bluetooth pairing
is a transport precondition only" becomes untrue: it is not a precondition at
all. The rest of that section is unaffected — per-pair Bluetooth enablement,
the untrusted-until-auth rule, and disabling semantics all stand, and the
second half of line 29 ("Fini app pairing remains the trust boundary") becomes
the whole of it.

**Confidentiality becomes an explicit, owned problem** rather than something we
assumed we had. Today we have none on either transport; this ADR does not add
it, it names where it goes.

**A new failure mode to design for:** connecting to whoever advertises means
connecting to strangers running Fini. App auth rejects them, but the attempt
costs a dial. Bounding that is part of the implementation, not an afterthought.

## Verification

- Hardware, the case that motivated this: with **no bond at all** between the
  two devices, a session establishes over Bluetooth, both sides report
  `session_transport == "bluetooth"`, and a quest converges both ways. That is
  `happy-path-BLE`, which exists and is red today.
- The same run with the phone's Wi-Fi off, so Bluetooth is provably the only
  path.
- A deliberately wrong peer must still be rejected: point one actor at a
  stranger advertising the same service and confirm app auth refuses it.

## Open questions

- Do we keep `paired_devices.bluetooth_address` at all? It stops being a dial
  input; it may still be worth showing in diagnostics, but a stored address
  that nothing depends on tends to grow dependents again.
- How many concurrent dials to unknown advertisers are acceptable before that
  becomes a battery or radio-contention problem, and what bounds them.
- Which protocol `secure_channel` should carry. Noise is the obvious candidate
  for a two-party channel with pre-shared identity, but that is its own
  decision and deserves its own record.
