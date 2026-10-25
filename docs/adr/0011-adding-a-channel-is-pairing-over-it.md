# 0011 — Adding a channel is pairing over it

## Status

Accepted. Written during the design discussion of 2026-10-25. Supersedes
ADR-0008 D1–D4.

## Context

ADR-0008 D1–D4 made the init of a channel a hello exchange in both
directions: `AddChannel` and `AddChannelAccept`. A device counted the init
from two facts that came in two different messages: "the peer accepted my
`AddChannel`" and "I accepted the peer's `AddChannel`". By D2, a device that
still had the channel row accepted an `AddChannel` with no setup open.

On hardware this gave a deadlock (the `bluetooth-channel-add-again` spec, run
2):

1. The phone pressed Add first and sent `AddChannel`.
2. The desktop had no setup open, but it still had its Bluetooth row. It
   accepted, and recorded nothing.
3. The phone counted that accept and stopped sending.
4. The desktop pressed Add. It waited for the phone's `AddChannel` in its
   own setup, which never came.

The two devices counted different facts from different messages.

Designs compared: iroh (trust is the `EndpointId`; any path that reaches it
is used, TLS is the proof), libp2p (`PeerId`, Noise/TLS on each transport),
KDE Connect (one paired device, many link providers), Syncthing (Device ID,
any allowed address), TCP (a state machine on one handshake), WebRTC ICE
(checks in both directions, triggered checks). None of them has a
separate init exchange for each transport.

## Decisions

**D1 — Adding a channel to a paired device is pairing over that channel.**
The dialog for a known device runs the pairing exchange of a new device,
over the channel that the person chose: `PairStart`, `PairAccept`, the code,
`PairComplete`. The list in the dialog shows only that device.

**D2 — The correct code writes the row.** When the person types the correct
code, both devices write the row of that channel as `On`
(`save_paired_device`, for the channel the pairing ran on). The device row,
its key and its spaces are kept. There is no OK button. Closing the dialog
before the code writes nothing.

**D3 — The pairing link carries the first session.** The link that the
pairing ran on stays up for the first session (`keep_pairing_link`), so the
row is green at once.

**D4 — Removed.** `AddChannel`, `AddChannelAccept`, `ChannelSetup` and its
two flags, `begin_channel_setup` and `end_channel_setup`, and the dial order
of setups are removed. The rules of ADR-0008 that are not D1–D4 stay: the
switch (D15), unlink (D14), and the row colours (D19).

## Consequences

- One code path for "pair" and "add a channel", proven by the
  `bluetooth-pair-from-zero` spec.
- Adding a channel takes one step more than before: the code. Both people
  agree to the channel explicitly.
- No wire message is added.
