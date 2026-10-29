# 0011 — A channel is a switch on each device

## Status

Accepted. Written during the design discussion of 2026-10-25. Supersedes
ADR-0008 D1–D4 (the init of a channel) and D14 (unlink).

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

Two other designs were tried in the same discussion: pairing over the new
channel with a code, and then pairing with Accept and no code. The second
one needs a request over Bluetooth in each direction; with the phone as the
sender, the answers did not reach it.

Designs compared: iroh (trust is the `EndpointId`; any path that reaches it
is used, TLS is the proof), libp2p (`PeerId`, Noise/TLS on each transport),
KDE Connect (one paired device, many link providers), Syncthing (Device ID,
any allowed address), TCP (a state machine on one handshake), WebRTC ICE
(checks in both directions). None of them has a separate init exchange for
each transport.

## Decisions

**D1 — Each pair has the two channels, each with a switch on each device.**
There is no Add, no Unlink and no setup for a channel. A switch is local to
its device: it is `On` or `Off`.

**D2 — A new pair has the channel of its pairing `On`.** The other channel
is `Off` (`save_paired_device`).

**D3 — A session on a channel needs the channel `On` on the two devices.**
iroh's TLS proves the pinned key. The receiving device refuses a session on
a channel that is `Off` there (close code `CHANNEL_OFF`). The sending device
keeps its switch `On`; its row is grey and tells that the channel is off on
the other device. When the person switches it on there, the next session is
accepted and the row is green.

**D4 — Removed.** Unlink and the `ChannelUnlinked` notice; the close code
`CHANNEL_MISSING` and the removal of a row that it caused; `AddChannel`,
`AddChannelAccept`, `ChannelSetup` and its flags, `begin_channel_setup`,
`end_channel_setup`, the dial order of setups; and the dialog that added a
channel to a paired device. Pairing a new device stays as it is.

## Consequences

- Least code: no message is added, and several are removed.
- Consent for a channel is "the switch is `On` on the two devices".
- A device with a switch `On` advertises and searches for the other device
  until the other device switches on too. The search backs off as in
  ADR-0008 D12. The device that switches on second searches at once, so the
  10 s standard still holds.
