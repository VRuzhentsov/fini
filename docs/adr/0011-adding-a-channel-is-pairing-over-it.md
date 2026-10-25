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
Both people open the dialog for the channel (Add on the row). Both devices
are then in Add Device mode for that channel: they advertise and scan, as
for a new device. The list on each device shows only the other device. The
person on one device taps it; the other device shows the request, with
Accept and Decline. The frames are the pairing frames: `PairStart`, the
accept, `PairComplete`. It works with no Wi-Fi.

**D2 — iroh's TLS proves the device; Accept writes the row.** The request
runs on a link of the chosen channel, and TLS on that link proves the key of
the peer. When that key is the key pinned for the pair, no code is shown:
Accept is the consent. Both devices then write the row of that channel as
`On` (`save_paired_device`, for the channel the pairing ran on). The device
row, its key and its spaces are kept. There is no OK button. Closing the
dialog before Accept writes nothing. When the key is not the pinned one (for
example, the app was installed again), the device is not known: pairing with
the code runs, as for a new device.

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
- Both people agree to the channel explicitly: one taps the device, the
  other accepts. The pinned key replaces the code.
- No wire message is added.
- Both devices must have the dialog open: a device with no dialog open does
  not advertise for the request.
