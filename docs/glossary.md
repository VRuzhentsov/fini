# Glossary

What Fini's words mean. When a word here conflicts with what the code says,
this file wins and the code is wrong.

For *how* names are formed — plurals, casing, command prefixes — see
[`naming.md`](naming.md). This file is about meaning, that one is about
spelling.

**Parentheses convention.** Where Fini's word for something has a more
familiar synonym elsewhere in the industry, documentation writes it as
`Term (Synonym)` on first mention — `channel (transport)`,
`DataLink (Connection)`. The synonym is there so a reader arriving with
other vocabulary lands in the right place; it is not a second name. After
the first mention, and everywhere in code, only the first word is used.

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

**What is spoken over a channel** — WebSocket today, potentially HTTP/3 or
something else later, *without the channel changing*. A person chose
"Network"; they did not choose "Network over WebSocket", and swapping the
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
have nothing in common — `TcpWsDataLink` keeps WebSocket ping counters,
`BleDataLink` keeps a GATT handle and a MAC. The commonality is the
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
at all, and routinely does. A stranger dials in, sends `PairRequest`, gets an
answer, and the pipe closes. That is the ordinary pairing path, and it needs
a working pipe *before* anyone is authenticated.

### How they read together

```
channel      A and B agreed to talk over the network        (a row, persisted)
protocol     ...speaking WebSocket                          (which dialect)
DataLink     ...and one is open right now to 10.0.0.7       (live, anonymous)
session      ...carrying an authenticated conversation      (live, identified)
```

## The messages themselves

### PeerFrame

**One message two devices send each other:** `PairRequest`, `Auth`,
`SyncEvent`, `Ack`, `Ping`. The application protocol, identical over every
channel — one copy of it, which is the whole reason `DataLink` exists.

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
| **stream** | implies a byte stream; ADR-0001 chose datagrams, because WebSocket, BLE and LoRa are all message-shaped |
| **peering**, **route** | structurally accurate but router-engineer jargon, and a channel appears in a settings screen |
| **transport**, as a separate layer | retired — it is simply the older word for *channel* |

### Radio

How the Bluetooth channel reaches a peer, injected into
`BluetoothChannelService` rather than reached for. `GattRadio` on a real
device; `LoopbackRadio` — a TCP connection to `127.0.0.1` — where there is no
hardware, which is CI.

`LoopbackRadio` was called "Sim", and the rename is the point: it read as a
fourth kind of channel beside Network and Bluetooth, which it never was. A
person cannot choose it, it appears in no table and on no screen, and it has
no discovery of its own. It is one way of connecting the Bluetooth channel,
and its links say so — `TransportKind::Bluetooth`, the same as any other.

It is not a mock. Links go through the same `DataLink`, codec, gate and
session loop as real ones, so a test over it exercises everything except the
radio. `ble-gatt`'s **mock broker** is the other CI stand-in and sits one
layer deeper: all the real BLE code, with only the radio faked. The two lanes
prove different things — loopback proves fallback works, the broker proves
Bluetooth works.

## Known gaps

`communication/channel/`'s `TransportKind` still spells "transport" in the
retired sense. It now has exactly two variants, `TcpWs` and `Bluetooth`, and
says precisely what `ChannelKind` says — one mechanical rename from being the
same type. It kept the old name only because the rename is a separate,
mechanical commit.

`LinkState`/`LinkEvent` (ADR-0005) keep the bare word `link`. They are the
state machine for whether a pair *has* a `DataLink` on a channel and what is
being done about it — a different thing from the pipe itself, and renamed
only if that turns out to confuse someone.
