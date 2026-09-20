# 0007 — Channels: how two devices connect and stay in sync

## Status

Accepted. Issues #171, #175. Supersedes the separate sync and device-page
decisions previously numbered 0007 and 0008.

## Context

Fini is local-first. Two devices a person owns sync directly, with no server:
a handful of quest changes a day, over the local network or over BLE. One of
them is usually a phone on battery.

Two devices cannot sync until at least one connection between them is
configured. There are two that can be configured, and a pair may have both
at once.

A person connecting their own devices has three questions, and the app owes
them an answer to each: **is it connected, has my stuff synced, and if not,
why not.**

## Decision

### Channels

**A connection medium is a Channel.** "Transport" is a networking word; the
code may keep it, the interface does not.

**A channel is configured, not inferred.** Setting up a connection asks which
of the two to configure. Discovery runs while a connection is being set up —
it is what finds the peer to configure against. Choosing automatically is a
separate feature layered on top (#176), never the default that hides this
one.

**Channels are data, not columns.** `communication_channels` names the
channels that exist; a device↔channel relation carries, per pair: whether the
channel is enabled, whether it is the primary, and any address the channel
learned. A new channel is a row, not a migration.

**A relation row means "configured".** It appears when the channel is first
set up, which gives a pair three distinct states per channel:

| State | Meaning |
|---|---|
| no row | never set up — the page offers to set it up |
| `enabled = false` | set up, switched off |
| `enabled = true` | on |

**Turning a channel off is not forgetting it.** Off stops this device
dialling, refuses the peer's inbound dial, closes any open session, and
clears the primary. What the channel learned is kept, so turning it back on
does not start from nothing. Forgetting is a separate act — *unlink channel*
— available only once the channel is off.

**Trust is established once, for the pair — not per channel.** Adding a
second channel to a paired device needs no passcode and does not interrupt
the other person: the devices exchange what they need over the channel they
already trust.

**A new pair's channels reflect how it was set up.** Pairing over Bluetooth
does not configure Network.

**The primary is a setting, not a live state.** It records which channel the
person chose to carry the traffic, and it persists — so it still governs
after a reconnect, and the page shows it whether or not that channel is
connected at this moment.

### Sync

**Sync is event-driven. Timers are a backstop, not a mechanism.** Work moves
when something makes it possible, and five moments do:

| Moment | Why |
|---|---|
| A local change | creates work |
| An inbound frame | creates work |
| A session is claimed | queued work becomes sendable |
| A channel is switched on | a peer becomes eligible again |
| A peer becomes reachable | dialling becomes possible |

Each fires on a **transition** only. A signal on the steady state — an
advertisement seen every scan window, an mDNS name re-resolving — would be a
poll wearing a different name.

**The backend owns the cadence.** One keeper task ticks on a timer, on every
platform. The frontend renders and listens; it drives no work and holds no
timer the backend depends on.

**The backstop is one hour**, covering a notification that never arrived and
nothing else. Reconnection does not depend on it, because a peer becoming
reachable is one of the five moments above.

**A peer's change reaches the UI as an event.** The frontend re-reads through
the commands it already uses; the event carries no payload, because what
changed is in SQLite and a second source of truth would buy nothing.

**Liveness is proven by the transport.** A dropped link raises
`SessionEnded` immediately. The app-level ping covers only what the transport
cannot see: a peer whose link is up but whose app has stopped answering.

### What the person sees

**Pairing is a modal**, reused for adding a device and for adding a channel
to an existing one. It is a short two-person ceremony where one side waits
while the other acts, and both screens say whose turn it is. Device identity
verification belongs here. There is no add-device page and no route standing
in for one.

**Every channel row says why, in plain language, naming the device.** "Pixel
8 isn't nearby" and "Bluetooth is off on this computer" are different
problems about different machines, and only one can be acted on where the
person is standing. Never a status code, never a bare coloured dot.

**Switching a channel on never fails and never reverts.** If the condition it
needs is absent, the channel stays on, says so, and starts by itself once the
condition clears.

**The device page answers the three questions and nothing else:** the
channels, the shared spaces, a sync queue, and unlink. The sync queue is its
own section — a count, and expanded, the titles of the Quests waiting, ten
then "N+ more". Unlink names the spaces that stop syncing and says nothing is
deleted.

**The devices list stays a list**: a coloured status circle, the name, and an
information icon carrying the detail.

**Power and lifecycle are not user-facing.** No connection policy, no quiet
hours, no per-device battery controls.

## Consequences

An idle, connected pair wakes on the hourly backstop and the ping. Everything
else it does is in response to something real. Latency is bounded by the
event rather than an interval.

Every wake source has to be correct, because the backstop is too far away to
hide a missing one. That is deliberate: it makes a missing wake visible as a
stalled pair rather than a slow one.

Adding a channel — LoRa, or anything else — is a seeded row and a transport
implementation, with no schema change and no new per-channel column.

## Not in this decision

- Negotiating a long BLE connection interval, the largest remaining lever for
  battery: needs `ble-gatt` to expose the parameter.
- Inline rename of a device (#117).
- Automatic channel selection (#176).
- Telling the requester that a pair request was declined: the protocol
  carries no such signal (#177).
