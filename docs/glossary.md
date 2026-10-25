# Glossary

What Fini's words mean. When a word here conflicts with what the code says,
this file wins and the code is wrong.

For *how* Fini writes — STE, the dictionary check, and how names are
formed — see [`naming.md`](naming.md). This file is about meaning, that one
is about writing.

**STE company glossary.** Fini writes in STE (see [`naming.md`](naming.md)). This
file is the company glossary that STE rules 1.5 to 1.13 refer to: the list
of Fini's technical nouns and technical verbs. A technical term that is not
in this file is not approved. Add a term only after you get the approval
of the project owner.

### Project owner

The person who makes the decisions for Fini. New terms and new files need
the approval of the project owner.

**Parentheses convention.** Where Fini's word for something has a more
familiar synonym elsewhere in the industry, documentation writes it as
`Term (Synonym)` on first mention — `channel (transport)`,
`DataLink (Connection)`. The synonym is there so a reader arriving with
other vocabulary lands in the right place; it is not a second name. After
the first mention, and everywhere in code, only the first word is used.

**Elsewhere line.** Each term ends with an *Elsewhere* line: the nearest
equivalent in other systems or designs, and how it is different. A reader
who knows the other design then knows where the term fits.

## Code design words

The roles from [`code-design.md`](architecture/code-design.md).

### Service

**An object that does one job and owns the state of that job**, for example
a channel or SpaceSync. Other parts call it through its interface. It does
the work; it is not a place where data is stored.

*Elsewhere:* a service in Spring, NestJS and Android; a manager in older
designs.

### Repository

**The only object that reads and writes one kind of stored data**, for
example the pairs or the outbox. A service gets data from a repository, not
from the database.

*Elsewhere:* a repository in DDD and Android; a DAO (data access object); a
store in the Matrix SDK; a registry in Home Assistant.

### Model

**The data of one thing, with no behavior that uses other parts**, for
example `Device` or `Quest`. A repository reads and writes models.

*Elsewhere:* an entity in DDD and in JPA; a record or a row.

### Command

**A request from the UI to do one thing**, with a name and arguments: a
Tauri command, for example `device_connection_unpair`. It gives back one
answer. Only the webview
calls Tauri commands (the UI, and the e2e tests through the devtools
bridge). The CLI and background tasks call the services directly.

*Elsewhere:* a controller action in MVC; an endpoint in a web API; a remote
procedure call (RPC). It is not PubSub: a command has one receiver and an
answer, and an event has many subscribers and no answer.

### Event

**A message that tells that a state changed**, for example a connection is
up. A service publishes it on the event bus. Each part that needs it
subscribes. The publisher does not know the subscribers.

*Elsewhere:* an event in PubSub, Syncthing's `lib/events`, Home Assistant's
event bus; a signal in Qt.

## Two devices reaching each other

These five are the ones most often confused, because several of them sound
like "a connection". They are not interchangeable.

### Channel (transport)

**The configured path between two specific devices.** What a pair *has*, and
the only one of these words a person ever sees.

One concept, two words. "Transport" is the older networking term for it,
written in parentheses on first mention so both readings land —
`channel (transport)`. Everything after that says *channel*: interfaces,
commands, tables, events, UI copy and prose.

A channel is per-pair, not per-app. A row in `channels` is keyed
`(device_id, channel_kind)`, so A↔B's Network channel and A↔C's Network
channel are separate things, separately switched on and off, separately
chosen as primary. It is persisted, it survives restarts, and it exists
whether or not anything is connected right now.

### Channel kind

**Which channel it is:** `network` or `bluetooth`, later `wifi_direct` or
`lora`. Seeded as rows in `channel_kinds`, so adding one is an `INSERT`
rather than a migration.

### Protocol

**What is spoken over a channel** — QUIC over iroh today (ADR-0009; it was
WebSocket until then), something else later, *without the channel
changing*. A person chose "Network"; they did not choose "Network over
QUIC", and swapping the
protocol must not look to them like losing a channel and gaining a different
one.

Not in the schema yet: there is only one protocol per channel kind today,
and a column with a single possible value is one nobody can get right.
It becomes a column on `channels` when a second protocol exists.

### DataLink (Connection)

**One open connection to one peer.** The byte pipe: two methods, `send` and
`recv`, whole payloads, boundaries preserved.

Named for the **OSI data link layer** (layer 2), which is exactly this job:
carrying frames between two directly connected nodes, independent of what
physically carries them — copper, fibre or radio. That independence is the
one property this abstraction needs, which is why the layer's own name is
the right one.

`DataLink` is a trait, so it holds **no data of its own**. Each
implementation stores whatever its own connection code requires, and they
have nothing in common — `IrohDataLink` keeps a QUIC stream and the
peer's proven key, `BleDataLink` keeps a GATT handle and a MAC. The commonality is the
*capability*, not the state.

Deliberately **not** called `Socket`: a BLE characteristic is not a socket,
and one of the implementations is exactly that.

### Session

**An authenticated conversation with a known device**, running on top of one
`DataLink`. Where `PeerSession` appears, it is this.

The difference from a `DataLink` is **identity, not connectivity**. A
`DataLink` is already connected when it exists — the TCP handshake or the
GATT subscription has completed. What is unknown is *who* is on the far end.
An IP address is not an identity; anything on the LAN can dial that port.

The proof they are different things: a `DataLink` can exist with no session
at all, and routinely does. A stranger dials in, sends `PairStart`, gets an
answer, and the pipe closes. That is the ordinary pairing path, and it needs
a working pipe *before* anyone is authenticated.

The identity comes from the TLS handshake: iroh proves the key of each side,
and the gate finds the paired device whose pinned key it is. There is no
auth message. A session runs only on the ALPN `fini/sync/<version>`.

A device has at most one session with a paired device on each channel kind
(`DeviceConnectionState::try_claim_session`). A session stays on the channel
that it started on until it ends; it does not move to a different channel.

### Secure channel

**The encryption seam between the `PeerFrame` codec and a `DataLink`**
(`channel/encryption.rs`, `SecureChannel`). `PlaintextChannel` gives the bytes
through without a change, and it is the only implementation. The frame
envelope records the scheme (`enc`), so a real encryption scheme can be added
later without a change that breaks the wire format.

### Link

**The radio connection between the Bluetooth chips of two devices** (a BLE
link). One link carries one `DataLink`, or the iroh connection of a session.
The link and the iroh connection on it can end at different times: Fini
closes the link itself when it does not need it (`close_links_to`). Two chips
can hold only one link between them. ble-gatt uses the same word
(`BleGattTransport::close_link`).

### Setup

**The steps that add a channel to a pair** (`ChannelSetup`,
`DeviceSetupDialog.vue`). The person presses Add on the channel row on the
two devices. The devices find each other and send `AddChannel`. OK switches
the channel on. A setup is open from Add until OK or Close.

### Green

**A channel is green for a device while it holds a live connection with a
session to that device** (`ChannelService::is_green`). A device that was only
heard — an mDNS record, a Bluetooth advertisement — is not green. When the
connection ends, the channel turns grey and connects again by itself.

### Gate

**The rules for who may connect** (`pairing/gate.rs`), in one place for all
channels. iroh's `Router` gives each new connection to the gate (`Gate`, an
iroh `ProtocolHandler`). The gate refuses with a QUIC close code; see
`docs/architecture/communication.md`.

### SpaceSync

**The object that carries the changes of mapped spaces to paired devices,
and the feature itself** (`sync/space_sync.rs`). It runs the sessions on the
links that the channels hold, and sends each change over the first green
channel of the pair (the primary first). Work for a device with no green
channel waits.

### How they read together

```
channel      A and B agreed to talk over the network        (a row, persisted)
protocol     ...speaking QUIC                               (which dialect)
DataLink     ...and one is open right now to 10.0.0.7       (live, anonymous)
session      ...carrying an authenticated conversation      (live, identified)
```

## The messages themselves

### PeerFrame

**One message two devices send each other:** `PairStart`, `SyncEvent`,
`Ack`, `SpaceMappingUpdate`. The application protocol, identical over every
channel — one copy of it, which is the whole reason `DataLink` exists.

### Messages that find and pair devices

Each message name is a technical noun of the Fini protocol.

| Message | Sent by | Tells the other device |
|---|---|---|
| `IdentifyDevice` | A device in Add Device mode | "This is my identity. Tell me yours." |
| `DeviceIdentified` | The device that received `IdentifyDevice` | "This is my identity." |
| `AddChannel` | A device that adds a channel to a pair | "I add this channel to our pair." |
| `AddChannelAccept` | The other device of the pair | "I add this channel too." |
| `PairStart` | The device on which the person pushes Pair | "Pair with me." |
| `PairAccept` | The other device | "I accept the pair." |
| `PairComplete` | The first device | "The code agrees. The pair is complete." |

## Finding and connecting to a device

### Adapter

The Bluetooth radio of this device, as the `ble-gatt` library gives it
(`ble_gatt::Adapter`). Fini gets the adapter from `adapter()`. The word
"backend" stays only inside `ble-gatt`, where a platform driver connects.

### Scanner

The part of Fini that receives Bluetooth advertisements. One scanner runs
for the whole app. It also keeps what it heard: the devices that are
available now, and when and where each paired device was last heard.

### Advertiser

The part of Fini that sends this device's Bluetooth advertisement, so that
other devices can find it.

### Add Device mode

The time when the Add Device screen is open. In this mode the device
advertises a flag that tells other devices it is in Add Device mode. In
code: `add_device_mode`.

### Available device

A device that you can pair with now: it is near, it is in Add Device mode,
and it has sent its identity. The Add Device screen shows available
devices. In code: `AvailableDevice`.

### Identify

To connect to a device in Add Device mode and get its identity
(`IdentifyDevice` and `DeviceIdentified`). In code: `identify_device`.

### Connect

To open a connection to a device. Fini does not use "dial".

### Pairing step

One part of a pairing: the start, the accept, or the complete. While a
pairing step runs, the scanner does not scan. In code: `PairingStep`.

### Connect guard

A value that holds the radio while a connection opens. When the program
releases the guard, the scanner can scan again. In code: `ConnectGuard`.

### Scan lock

The lock that lets only one scan run at a time. In code: `scan_lock`.

### Frame envelope

The versioned wrapper around an encoded `PeerFrame`: `{ v, enc, payload }`.
It exists so switching real encryption on later is additive rather than
breaking — every frame ever sent already carries `v: 1` and `enc: none`, so
an older device can say "I don't know that scheme" instead of choking on
what looks like garbage.

### Framing

**Where one blob ends** on a raw byte pipe: a 4-byte length prefix, or BLE's
fragments. Unrelated to `PeerFrame` despite the shared spelling, and the
distance between them is not academic — one ~400-byte `SyncEvent`, a single
`PeerFrame`, once landed on BLE as 238 fragments taking 20+ seconds.

## Other words

### Primary

The channel the person chose to carry a pair's traffic. A **setting, not a
live state**: it persists, it governs the next reconnect, and the page shows
it whether or not that channel is connected at this moment. `is_primary` in
SQL only because `primary` is a keyword.

### Install channel

Unrelated to everything above: the app-update track (stable/beta).

## Words deliberately not used

| Not used | Why |
|---|---|
| **socket** | a BLE characteristic is not one, and one `DataLink` is exactly that |
| **stream** | implies a byte stream; ADR-0001 chose datagrams, because WebSocket, BLE and LoRa are all message-shaped (`IrohDataLink` frames its QUIC stream to keep that contract) |
| **peering**, **route** | structurally accurate but router-engineer jargon, and a channel appears in a settings screen |
| **transport**, as a separate layer | retired — it is simply the older word for *channel* |
| **arbiter** (`RadioArbiter`) | jargon (STE rule 1.10); the object is the *scanner* |
| **directory** (`PeerDirectory`) | means a file folder; the data belongs to the *scanner* |
| **backend** | does not say what it is; Fini uses *adapter* |
| **probe** | not an approved word; Fini uses *identify* |
| **candidate** (`AddModeCandidate`) | not an approved word; Fini uses *available device* |
| **dial** | not an approved word; Fini uses *connect* |
| **leg** (`PairingLeg`) | jargon; Fini uses *pairing step* |
| **lease** (scan lease) | jargon; Fini uses *scan lock* |
| **hello** (`Hello`, `DiscoveryHello`) | not an approved word; the messages are `AddChannel` and `IdentifyDevice` |
| **request** (`PairRequest`) | not an approved noun; the message is `PairStart` |
| **note** (verb) | not an approved verb; Fini uses *record* |
| **refuse**, **refusal** | not in the STE dictionary; Fini uses *reject*, *rejection* |
| **take** | not an approved verb; Fini uses *remove* (or *read*, *get*) |
| **within** | not an approved preposition; Fini uses *in* |
| **delay** (noun) | not an approved noun; Fini uses *time* |
| **cooldown** | not in the STE dictionary; Fini uses *wait time* |
| **window** (a time) | not in the STE dictionary; Fini uses *time* |
| **Peer Protocol** | the older name for the frames between two devices; Fini says *`PeerFrame`* and *session* |
| **Connection Manager** | an informal name for the session claim and the channel order; no object has this name |
| **sticky handoff**, **transport selection** | older names for the rule of one session for each channel (see *Session*) and for the channel order (see *Primary*) |
| **`Auth`**, **`AuthOk`**, **`AuthFail`** | removed frames; the TLS handshake proves the key (see *Session*) |

### Radio

How the Bluetooth channel reaches a peer, injected into
`BluetoothChannelService` rather than reached for. `GattRadio`, via
`ble-gatt`, is the only one.

There was a second — a TCP stand-in on `127.0.0.1`, called "Sim" and then
`LoopbackRadio` — for CI, which has no hardware. It is gone. Renaming it
never fixed what was wrong with it: it still read as a third kind of channel
beside Network and Bluetooth, it still had to be carried through every
switch and table that talks about channels, and it still invited the
question "which one am I on" that the whole channels model exists to remove.

`ble-gatt`'s **mock broker** covers CI properly, and one layer deeper: it
fakes the radio underneath `ble`, so the dial loop, the peripheral accept,
the session claim and the GATT roles above it are all the real code. That is
a better proof than a stand-in channel was, and it costs nothing in the
product.

## Known gaps

`LinkState`/`LinkEvent` (ADR-0005) use the word `link` for a different
thing than the term **Link** above: the state machine for whether a pair
*has* a `DataLink` on a channel and what is being done about it. They are
renamed only if that turns out to confuse someone.
