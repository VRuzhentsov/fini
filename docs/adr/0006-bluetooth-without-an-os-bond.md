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
   works. Note that `probe_candidate` (`ble.rs:521`) already does exactly this
   — dial a scanned candidate and run `perform_client_auth` to learn who it is
   — so identification by advertisement is not new code, it is code we already
   run and then throw away.
3. **Drop the bond checks.** `check_bluetooth_bond` and the OS-pairing arm of
   `bluetooth_dial_candidates` go away, along with `bluetooth_address` as a
   dial input and as a precondition. The column itself stays, written as
   "where we last saw this peer" for diagnostics — see the design review below.
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

## Design review

The decision above leaves one thing unanswered that turns out to be
load-bearing: **who dials.** `should_dial_peer` picks a side by comparing
`self.device_id < peer.device_id`, and under this ADR we do not learn the
peer's `device_id` until after we have connected and authenticated. The rule
loses its input exactly when it is needed.

Six decisions close that gap and the questions behind it.

**1. A short identity fingerprint rides in the advertisement.** Four bytes
derived from `device_id`, carried in the manufacturer data that
`datagram_config` already populates and the scanner at `ble.rs:684` already
reads. A legacy advertisement holds 31 bytes and the 128-bit service UUID plus
the existing manufacturer record spend about 26, so this fits with room to
spare. `should_dial_peer` keeps its rule and compares fingerprints instead of
full ids. A fingerprint collision only means both sides dial, which is glare —
a case the machine must survive regardless.

The payload stops being a single flag byte, so the exact-equality test at
`ble.rs:684` (`== Some([ADD_MODE_FLAG_BYTE])`) becomes a field read. That
comparison is the one place a careless change silently disables add-mode
discovery.

**2. The fingerprint is stable now and rotating later, deliberately.** A
constant identifier broadcast continuously is trackable, and it undoes the
address rotation Android performs on purpose. The privacy-correct answer is a
fingerprint derived from a per-pair secret and a coarse time window, so only a
paired peer can recognise it — but `paired_devices` (`schema.rs:92`) stores no
key material at all, so that answer is not available today. The same missing
ingredient blocks `secure_channel`. Both get it at once; until then this is
recorded as known debt rather than an oversight.

Worth stating plainly: the service UUID already makes a device identifiable as
"some Fini install". The fingerprint raises that to "this particular install".

**3. "Not seen advertising" is a precondition, not a silent wait.** It maps
onto the existing `PreconditionsLost { reason }` event, so the machine needs no
new state, and it takes over the code slot that `BluetoothNotOsPaired` vacates.
The row goes gray with an honest reason. The alternative — sitting in `Idle`
showing amber "connecting…" at a peer who is in another building — is the exact
dishonesty ADR-0005 exists to remove.

**4. Scanning is duty-cycled, and the cycle depends on who is watching.** A 5s
listening window, repeated every 30s in the foreground and every 60s in the
background daemon: the foreground pays for a row that turns green while the
user is looking at it, the daemon pays for battery. Scanning pauses while a
session is live, because the link already proves presence.

Continuous scanning is rejected on first-hand evidence, not theory: sustained
scanning on the development desktop destabilised the host adapter badly enough
to drop the machine's unrelated Bluetooth devices, recorded under ADR-0005's
method traps.

The freshness window for decision 3 follows from the period rather than being
chosen: it must span several cycles, or one missed beacon drops the row
spuriously. Roughly 90s in the foreground, roughly three minutes in the
background. The cost is honest but unhurried: after a peer really leaves, the
row can take up to three minutes to go gray.

**5. `find_peer_address`/`probe_candidate` are promoted, not replaced.** They
already scan, dial and authenticate; they simply discard the established link
and return an address to redial. That discard is the address-centric model's
last artefact. A successful probe *is* the session, so it keeps its link. The
fingerprint becomes a cheap pre-filter deciding which candidates are worth
probing at all — which is also what bounds dialling to strangers, since an
unrecognised fingerprint is never probed.

**6. The work lands in slices, dial-by-advertisement first.** Only that slice
turns the manual happy path green, and it is the slice that tests this ADR's
central claim — that a link establishes with no bond of any kind. ADR-0005's
own Risks section argues the general case: landing in one piece leaves no
intermediate version to verify on hardware, "and hardware verification is what
found every defect in this area."

## Security consequence, stated plainly

The Context table above says the bond bought no trust, citing
`specs/device-connect/README.md`. That is right about *design intent* and
incomplete about *practical effect*, and issue #162 makes the sharper point:
because the `Auth` frame proves nothing cryptographically, `check_bluetooth_bond`
was in practice the only mechanism standing between a Bluetooth peer and
impersonation of a paired device. Removing it removes that mechanism.

Three things make this an acceptable trade rather than a regression, and all
three should be checked before anyone relies on the reasoning:

1. **It was weak.** The check asked whether the connecting device presented an
   address we had bonded. BLE addresses are trivially spoofable, so it stopped
   an attacker who did not know the peer's identity address and nobody else.
2. **The primary transport never had it.** `tcp_ws` accepts a plaintext
   `device_id` claim with no equivalent check at all. Bluetooth is the
   secondary transport; this brings it to parity with the primary one rather
   than opening a new class of exposure.
3. **The real fix is already tracked.** Issue #162 authenticates the `Auth`
   handshake cryptographically for *every* transport, which fixes both at once.

What this ADR changes about #162 is its ordering, not its content: that ticket
was written expecting to land *before* the bond could be dropped, and the bond
has been dropped first. Its priority should rise accordingly.

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

Three of the original questions were closed by the design review above: the
stored address stays as diagnostics only, dialling is bounded by the
fingerprint pre-filter, and the dialer is chosen by comparing fingerprints.

Still open:

- Which protocol `secure_channel` should carry. Noise is the obvious candidate
  for a two-party channel with pre-shared identity, but that is its own
  decision and deserves its own record. It needs the same per-pair key material
  that a rotating fingerprint needs, so the two should be decided together.
- Whether `bluetooth_address` survives contact with decision 6. It is kept for
  diagnostics on the grounds that hardware debugging goes blind without it, but
  a stored address nothing depends on tends to grow dependents again. Worth
  re-checking once the slices land.
- The exact fingerprint derivation. Four bytes of a hash of `device_id` is the
  assumption; whether that is a plain truncation and which hash is unresolved,
  and matters only for interoperability between versions.
