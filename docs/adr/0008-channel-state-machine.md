# 0008 — Channel state machine: init, presence, and a battery-first background service

## Status

Accepted. Written during the design grill. Follow-ups: #183 (stale peers and unlink propagation), #184 (peer authentication).

## Context

Channel state lives in 4 independent stores (the `channels` table,
`DiscoveryRuntime`, the process-global statics in `channel/ble.rs`, and
`channelStatusesByPeer` in the Vue store). It is written from 6 Rust modules
and 4 frontend files, and no invariant is enforced across the stores. The
result is state combinations that mean nothing to the person, for example:

- A channel that is switched on but has never connected once. Nothing records
  a first success; `configured_at` is written and never read.
- "Find" reports a peer as found while that peer will refuse every session
  (gate.rs:124-143 against gate.rs:230-243).
- A channel whose peer has it off reports "Couldn't connect" and offers a
  "Try again" that cannot succeed.
- A row that switches between two orderings every 5s, because the frontend
  derives state a second time.

## Glossary

Main term first; synonyms in brackets mean the same thing and are not
separate mechanisms. Text in this ADR and in code uses the main term only.

| Term | Meaning | Replaces today's |
|---|---|---|
| **Background service** (daemon) | The app's always-running process (ADR-0004). Owns every timer and every radio action; the UI only renders. | tick keeper |
| **Advertising** (beacon, announce, presence broadcast) | One-way "this Fini device is here" signal on a channel. Nobody answers it. Cheap for the sender. Bluetooth: BLE advertisement. Network: mDNS/UDP announcement. LoRa (future): beacon. | advertise, DiscoveryHello, mDNS presence |
| **Search** (scan, scanning, discovery) | Turning the radio to receive and looking for other devices' advertising. This is the expensive part: the radio must stay on to hear anything. One mechanism with three purposes — setup, delivery, status — that only set its parameters (D12, D13). A search lasts up to 60s and ends early once it finds what it was started for (setup search collects everything in range instead). | presence scan, dial scan window, "Find" |
| **Listen** (subscription, subscribe, handler) | An event handler for incoming exchanges, like a JavaScript event listener or pub/sub subscription: when a peer pushes, the handler stores the data and answers with an ack. Costs nothing beyond advertising, because a device that advertises is already reachable. Not a radio activity. | gate, inbound session handling |
| **Presence** (available, online, green) | Derived, not stored: a paired peer's advertising was seen within the channel's **channel timeout**. | peer nearby, liveness, Connected, Fading |
| **Channel timeout** | How long a seen advertisement keeps a peer present. Per channel kind. | freshness (180s), FADE_GRACE |
| **Exchange** (push, send + acknowledgement) | Connect to a present peer, authenticate, send messages, get an acknowledgement for each, disconnect. The only way data moves. | session, dial, link, probe, ping/pong |
| **Acknowledgement** (ack, "200 OK") | The peer's reply that it stored a message. Only an ack removes an item from the queue. | ChannelEnabledAck, pong, ProbeReply |
| **Queue** (outbox) | Messages waiting for a peer. Pop, push, and remove on ack. | sync queue |
| **Init** (initialization) | The first exchange in both directions on a channel, done during active search. Sets `initialized`. | Find via Bluetooth |

## Decisions

**D0 — Battery comes first.** The background service must not drain the
battery. Earlier decisions on timers, ticks and refresh intervals are open to
revision wherever they conflict with this. Active phases (pairing, adding a
channel, search) may spend more; the steady state may not.

**D1 — Init is a hello exchange in both directions.** A (pair, Bluetooth)
channel becomes `initialized` on a device once that device has received an
ack to its own hello AND has acked the peer's hello. No extra message is
needed.

**D2 — Hellos are answered only while searching.** A device acks a hello
only while it is itself searching for that peer. Init is
therefore mutual by construction. It also closes the leak where an unlinked
channel still answers probes (inventory 4.3).
Once init has completed, a device also acks that peer's hellos while the
channel exists (`Off` or `On`). Its own ack can be lost when the link drops,
leaving the peer still searching after this side has finished; the peer's
retry must still get an answer. An unlinked channel has no row and stays
silent.

**D3 — `initialized` is cleared only by unlink or unpair.** Under D15,
`initialized` is simply "a row exists" (`Off` or `On`). It survives
switching the service off and on, restarts, and being out of range.
Re-adding an unlinked channel means a fresh mutual search.

**D4 — OK is local, and gated on init on both devices.** OK switches on the
Bluetooth service for this device only. It can be pressed only once init has
completed on both devices (each side can tell from D1). Nothing is switched on
remotely.

**D5 — Pairing initializes only the channel it happened over.** A pairing
completed over Network counts as Network init; one completed over Bluetooth
counts as Bluetooth init. There is no implicit setup through another
channel: a pair made over Bluetooth has no Network channel until someone
adds it in the setup dialog (D20), even when the two devices meet on a
network, and the reverse holds for Bluetooth.

**D6 — Adapter loss does not change the switch.** The Bluetooth service cannot
be switched on while Bluetooth does not work on this device. If the adapter
disappears while the service is on, the switch stays on, the row shows a
distinct "unavailable on this device" state, and the channel resumes by itself
when the adapter returns.

**D7 — Presence is the availability signal.** A device advertises only while
its Bluetooth service is on. A peer with the service off is simply not present:
not green, and the sync queue does not drain over Bluetooth. There is no
"switched off" announcement and no "Couldn't connect" error.

**D8 — The background service advertises when any Bluetooth channel is on,
and pushes when a peer appears.** The background service advertises while
this device has at least one Bluetooth channel on. It watches for paired
peers' advertisements. When one appears (the peer turns on Bluetooth or the
channel, or comes into range), it pushes the waiting queue. This works in
both directions and does not need the UI to be open.
It also keeps advertising for two minutes after a Bluetooth init completes
here, even if the channel was left off. The peer may have missed this
device's ack and still be searching (D2), and can only ask again if it can
find this device.

**D9 — Green means presence.** A peer is green on a channel when its
advertising was seen within that channel's timeout. Advertising means the
peer's background service is up with that channel on, so we expect a push to
be acknowledged.

**D10 — Pub/sub with acknowledgement.** Changes are published to the queue of
each subscribed peer. The background service takes an item, pushes it in an
exchange, and removes it only when the peer acknowledges it. Delivery is at
least once, so receivers must handle duplicates.

**D11 — Every channel follows the same model.** Network, Bluetooth and future
channels (LoRa) all use advertising, search, presence, exchange and ack. Only
the timings and the underlying transport differ.

**D12 — Search runs only when there is a reason to, and each search is a
full 60 seconds.** A search keeps the radio receiving for up to 60s and ends
early the moment it finds what it was started for (for delivery: the peer, so
the push starts at once). There is no short window followed by idle time.

- UI closed, nothing queued: no search. The radio only advertises and listens.
- UI open: status search runs continuously; channel timeout is 60s.
- Queued items for a peer that is not found: delivery search repeats after
  1, 5 and 15 minutes, then every 15 minutes, until the peer is found or the
  queue is empty.
- Setup: continuous while the dialog is open, and it does **not** stop at the
  first find: setup search is meant to show every Fini instance in range, so it
  keeps collecting until the person picks one or closes the dialog.

These numbers are the first thoughtful attempt at background battery
optimization, not measured values. Before the next round of tuning, profile
the real battery use on hardware (phone and desktop) and revise from data.

**D13 — Search is one channel-agnostic mechanism with purposes.** Every
channel (Network, Bluetooth, LoRa) exposes the same search interface; only
the underlying mechanics differ (mDNS/UDP, BLE scan, LoRa receive). Callers
ask for a search with a purpose — setup, delivery or status — and subscribe to
"peer seen" events. A single shared, in-memory search coordinator per channel
merges overlapping requests into one running search on the most demanding
parameters, so the radio is never started twice for the same thing.
Today's `ChannelService` trait (`channel/service.rs:43`) is the starting
point, but it is stateless and rebuilt on every call (`service.rs:136-141`),
and the real search state lives in process-global statics in `ble.rs`; the
coordinator is where that state moves to.

**D14 — Unlink is a hard delete, and the peer is told.** Unlinking a channel
deletes its row; there is no tombstone (`unlinked_at`, migration 24, goes
away). The tombstone existed so a peer could not recreate a removed channel;
under D1–D4 a channel can only be created by a mutual init, which needs both
people searching, so a peer cannot recreate it on its own. Unlinking also
tells the peer with an "unlinked" message. On receipt the peer deletes its
own row for that channel and stops delivery search for it.

The message is kept in the database (`peer_control_outbox`) until an
exchange has written it to the link, so neither a restart nor a dropped link
loses it. The same goes for "space sync ended": ending a sync, like
unlinking, is one-sided, and the peer must hear of it however long it is
away. A later act on the same thing replaces a message not yet sent: setting
the channel up again drops its unlink, and syncing the space again drops its
end. Delivered late, either would undo the newer act. Kept messages go with
the pairing when it is removed.

Sync events are not kept this way: what to send is worked out again from the
two devices' state on every exchange. A space sync request is not kept
either. The other person answers it at their device, so it goes while the
peer is reachable (an exchange running, or the peer present on an enabled
channel) and fails otherwise.

**D15 — The stored channel state is `None`, `Off` or `On`.** Per (pair,
channel). `None` is the absence of a row. Transitions, each guarded in the
data layer so no caller can create another combination:

| From | Action | To |
|---|---|---|
| `None` | init completes, OK pressed | `On` |
| `None` | init completes, OK not pressed | `Off` |
| `Off` | switch on | `On` |
| `On` | switch off | `Off` |
| `Off` | unlink, or "unlinked" received from the peer | `None` |
| any | unpair | `None` |

`On` is reachable only through init, so "on but never connected" cannot be
represented. Runtime facts (presence, adapter availability, queue length) are
not part of this state. The state machine is not a UI model; how the UI
presents it is a separate mapping, decided separately.

**D16 — Every exchange starts with authentication; its design is a separate
ADR.** Today "auth" is an unverified claim: the `Auth` frame carries only
`device_id`, `peer_device_id` and a protocol version
(`sync/types.rs:84-89`), the receiver checks the ID against its paired list
(`gate.rs:202-212`), and `paired_devices` stores no key material
(`schema.rs:92-98`). Any device that learns a peer's ID can push as that peer.
Real authentication (keys exchanged at pairing, each exchange proven with
them) is designed in its own ADR and tracked in its own ticket; this ADR only
requires that an exchange is refused unless authentication succeeds.

**D17 — Session machinery is deleted, not kept alongside.** Held sessions,
`LinkState` (`pairing/link_state.rs`), ping/pong liveness
(`peer_channel_ack`), the `ble.rs` dial backoff and `dial_exhausted`
statics, and the "Try again" action exist only to manage held sessions. They
are removed when exchanges replace sessions. The order of removal belongs in
the implementation plan, not here.

**D18 — Each channel implements advertising and search its own way.** The
model (advertising, search purposes, presence with a channel timeout,
exchange with ack) is shared through the channel service interface (D13).
How a channel advertises and searches, and its timings, belong to that
channel's implementation. Bluetooth uses BLE advertising and scanning with the
D12 numbers; Network uses its own mechanism (today mDNS and a UDP multicast
beacon, `pairing/runtime.rs`) and sets its own timings behind the same
interface.

**D19 — Channel row presentation.** The backend derives one presentation per
channel row from the D15 state plus runtime facts; the frontend only draws
it and never re-derives state.

| Colour | What it means | Stored state | Controls | ⓘ |
|---|---|---|---|---|
| Green | Peer seen within the channel timeout; data will go through | `On` | toggle | — |
| Grey | Peer not seen: out of range, or their channel is off | `On` | toggle | — |
| Orange | Problem on this device's channel (Bluetooth adapter off, a network issue, anything the channel reports) | `On` | toggle | popup explaining the problem |
| Empty circle | Channel switched off | `Off` | toggle, unlink | — |
| (no dot) | Channel not added | `None` | Add | — |

- ⓘ appears only on orange rows and opens a popup; there is no inline
  explanatory text on the row.
- There is no "Try again", "Connecting…" or "Fading".

**D20 — One setup dialog for every use case.** Adding a new device and adding
a channel to a known device use the same dialog, replacing today's
`PairDeviceDialog.vue` and `ChannelSetupDialog.vue`.

| # | Use case | Known about the peer beforehand |
|---|---|---|
| UC1 | Add a new device, never paired | Nothing |
| UC2 | Add a channel to an already-paired device | Its identity |
| UC3 | Re-add a channel after unlink | Same as UC2 |
| UC4 | Pair again after unpairing | Same as UC1 |

| Step | New device (UC1, UC4) | Known device (UC2, UC3) |
|---|---|---|
| 1. Channel | Choose Network or Bluetooth | Skipped: set by the row's Add |
| 2. Search | Live list of every Fini device in range | Same list; the known peer is marked and preselected |
| 3. Selection | Explicit pick | Preselected; another device can be picked |
| 4. Identity | Code ceremony (code shown on one side, entered on the other) | Automatic hello exchange (D1); the code ceremony only as a fallback when automatic confirmation fails |
| 5. Ready | OK available once both sides confirmed | Same |

Branches, for both columns:

| Situation | Result |
|---|---|
| The other device has not opened its dialog yet | The list keeps waiting; the search runs while the dialog is open (D12) |
| The peer declined | "Declined", back to the list |
| The peer closed its dialog midway | Back to the list; the peer drops out of it (D2) |
| Code does not match | Back to the code step |
| Dialog closed before step 5 | Nothing saved; state stays `None` |
| Dialog closed after step 5 without OK | `Off` (D15) |

There is no "nobody found" step, no "Try again", and no inline explanatory
text; hints are behind ⓘ.

## Addendum — Bluetooth on real hardware

Taking add-mode pairing and channel setup onto a Linux laptop and a Pixel
showed four ways the radio work above gets in its own way. None of them
changes a decision; they say how D12, D13 and D20 hold up on one adapter.

**The add-mode candidate scan stands aside during pairing legs and for the
whole Bluetooth setup.** The setup dialog runs add mode for a known device
too (D20). Its candidate scan holds a discovery session and dials every Fini
advertiser it hears, often the very phone that a pairing leg (request,
accept, complete) or the setup search is dialling. Crossing connections fail
each other's GATT setup. So each pairing leg and each Bluetooth setup holds
a `PairingLeg` (`channel/ble.rs`):

- A scan pass waits for every running leg to end before it starts.
- A pass that is still listening gives up as soon as a leg begins.
- A probe already dialling is let finish, not cancelled. Abandoning a dial
  makes ble-gatt quarantine the address and remove the device from BlueZ,
  which fails the dial that follows. The pass stops before its next probe.
- A pass that gives up returns an error, not an empty list, so the picker
  keeps the candidate being paired.

The setup holds its leg from start to end, not only until its own hello is
acknowledged: the peer's hello still has to reach this device afterwards.

**Setup dials are ordered by device id.** Both devices search for each
other and would dial the moment they hear each other. A device that is
itself mid-connect is not connectable, so the two dials hang until BlueZ
aborts them (8–12 s), and the retry crosses again. In a Bluetooth setup the
device with the lower id dials first. The one with the higher id waits until
the other's hello has reached it, or for at most 15 s, before it dials back.

**An address whose dial just failed is skipped for 10 s.** At the start of
each discovery BlueZ re-reports every device it has cached, including the
phone's previous private address. Dialling that stale entry, being refused,
and restarting discovery became a start/stop/dial loop every 2 s. That loop
left BlueZ's discovery wedged until bluetoothd restarted. The search now
skips a failed address for 10 s and keeps listening for the peer's current
one.

**On Linux the advertisement is registered again after the last central
leaves.** BlueZ stops transmitting the advertisement once a central that
connected to it has gone, while still reporting the instance as active, so
the peer never heard this device again. The peripheral re-registers it once
no accepted central is left. This is Linux only. On Android, registering
again gives the phone a new private address, so the address the other device
had just heard goes stale.

## Addendum (2026-10-05): links run over iroh

Fini ADR-0009 (D9) keeps this model whole and changes only what is under
`DataLink`:

- **Network**: one iroh endpoint per device, IP only, relays off, on the
  same port. Presence is still the mDNS/UDP beacon, which now also
  announces the device's key.
- **Bluetooth**: sessions (`Auth`, channel `Hello`) are iroh connections over
  `ble-gatt-iroh`; pre-pairing frames stay on plain GATT links. One GATT
  service carries both, routed by each channel's first datagram.
- **Trust**: the gate now accepts `Auth` and `Hello` only over a link that
  proved the key pinned for the pair (`paired_devices.endpoint_id`).
  Crossing dials, one exchange per (peer, channel), presence, init by mutual
  hello and the primary channel are unchanged.
- **Liveness**: QUIC's keep-alive and idle timeout replace the WebSocket
  ping; an idle Bluetooth channel closes after 20 s.

## Worth investigating later

- Two app instances fighting over one Bluetooth adapter. ADR-0005 recorded the
  desktop adapter becoming unstable and dropping the machine's other Bluetooth
  devices while two instances ran at once; `ble.rs:1393-1400` attributes it to
  continuous scanning instead. The cause is not proven. Desktop auto-update
  currently leaves the old instance running, which reproduces this condition.
- Network currently advertises through two mechanisms at once: mDNS
  (`pairing/runtime.rs:231-255`) and a UDP multicast beacon every 5s
  (`DISCOVERY_INTERVAL_MS`, `pairing/mod.rs:91`). Whether both are needed is
  for the Network implementation to settle under D18.

## Open questions

None at present.
