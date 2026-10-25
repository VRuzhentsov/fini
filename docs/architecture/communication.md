# Communication

How two paired devices find each other, connect, and sync. The code is in
`src-tauri/src/services/communication/`. The words are defined in
[`docs/glossary.md`](../glossary.md); how names are written is in
[`docs/naming.md`](../naming.md). This document is written in STE
([`docs/ste.md`](../ste.md)).

## The standard

Two devices find each other within 10 seconds under good conditions:

- **Start:** the search runs on the two devices. That is, the second device
  opens its dialog.
- **Stop:** each device shows the other in its list of devices that it can
  connect to. For a new pair, this is the Add Device list. For a new channel,
  this is the setup dialog of the channel, where OK becomes available.

This applies to Bluetooth and to Network, to a new pair (Add Device) and to a
new channel for a pair. Good conditions: the devices are near each other,
Bluetooth is on, Wi-Fi is as it is. A device that is slower under these
conditions has a defect in Fini.

The steps after the stop are not in the standard: the pair request, the
code, and the time until the channel is green. These steps must also be
fast, but they have no fixed limit.

## The modules

| Module | Owns |
|---|---|
| `pairing` | trust between two devices: Add Device, the pair, the channels of a pair, the gate |
| `channel` | the connections: one object per channel kind, each a pipe for bytes |
| `sync` | SpaceSync: what goes through the pipes, and when |

## Channels are pipes

A channel holds one live connection to each paired device that has the channel
on. A channel does not know what goes through the connection.

- **Green** is a live connection: an iroh connection that the TLS handshake
  proved, with a session on it (`ChannelService::is_green`). A device that
  was only heard (an mDNS record, a Bluetooth advertisement) is not green.
- **Network** (`channel/network.rs`, `NetworkChannel`): an iroh endpoint over
  IP. iroh's mDNS lookup finds the devices. QUIC's keep-alive holds the
  connection.
- **Bluetooth** (`channel/bluetooth/`, `BluetoothChannel`): an iroh endpoint
  over `ble-gatt-iroh`. A scan finds the devices (`Scanner`). The connection
  is held like the Network one.
- Each channel connects by itself (`ChannelService::keep_serving`, on each
  tick) to every paired device that has the channel on and has no session.
- A connection that hears nothing for 15 s ends (`iroh_link::transport_config`):
  a device whose radio went away turns grey after about 15 s.

```mermaid
flowchart LR
    subgraph CH["channel: one object per kind"]
        NET["NetworkChannel<br/>iroh endpoint over IP<br/>mDNS lookup"]
        BLE["BluetoothChannel<br/>iroh endpoint over ble-gatt-iroh<br/>Scanner, Advertiser"]
    end
    subgraph PA["pairing"]
        GATE["Gate: iroh ProtocolHandler<br/>fini/sync/4: key to paired device<br/>fini/peer/1: pairing, AddChannel, IdentifyDevice"]
        SETUP["setup: AddChannel / AddChannelAccept"]
        FLOW["flow: IdentifyDevice, PairStart / Accept / Complete"]
    end
    subgraph SY["sync"]
        SS["SpaceSync<br/>hold_session, wake, keeper,<br/>channel order (primary first)"]
        SESS["run_session<br/>one session per device and channel"]
        OUT["outbox (sqlite)"]
    end
    NET -- "own connection (SYNC_ALPN)" --> SS
    BLE -- "own connection (SYNC_ALPN)" --> SS
    NET -- "peer's connection" --> GATE
    BLE -- "peer's connection" --> GATE
    GATE -- "accepted" --> SS
    SS --> SESS
    OUT -- "tick: push over the first green channel" --> SS
    SETUP -- "connect_plain (PEER_ALPN)" --> CH
    FLOW -- "connect_heard_device / pairing links" --> CH
```

## Who is on the other end

There is no auth frame. iroh's TLS handshake proves the key of each side.
Pairing pins the other device's key (the PIN step). The gate finds the paired
device by the key that the handshake proved (`pairing::paired_device_for_key`).

Two ALPNs (the protocol name that the TLS handshake agrees on):

| ALPN | Carries |
|---|---|
| `fini/sync/4` (`SYNC_ALPN`) | a sync session. The last part is the protocol version: two builds on different versions do not connect. |
| `fini/peer/1` (`PEER_ALPN`) | the frames before a session: `PairStart`, `PairAccept`, `PairComplete`, `AddChannel`, `IdentifyDevice` |

Before pairing there is no key to pin. Over Bluetooth, the identify step and
the pair request first use a plain link (`BleDataLink`) or an iroh connection
to the key the device advertised. Pairing pins that key.

### Refusals

The gate refuses a session with a QUIC close code. The connecting side reads
it (`DataLink::close_code`).

| Code | Meaning | What the connecting side does |
|---|---|---|
| 1 | this key is pinned for no pair | waits, then tries again (`FAILURE_COOLDOWN`) |
| 2 | the channel is off here | waits, then tries again |
| 3 | crossing: the other device's connection is kept | nothing: that connection carries the work |
| 4 | a session runs already | nothing |

### Two connections at the same time

Both devices can connect to each other at the same moment. Each device then
keeps the connection that the device with the smaller id started, and closes
the other. Both devices apply the same rule, so they keep the same
connection. A device's own connection counts as "connecting" while it is made
and for 3 s after its session starts (`session::CROSSING_WINDOW`). A connection
from the peer after that time means that the peer started again: the old
session closes, and the new one replaces it.

```mermaid
flowchart TD
    IN["the peer connects (SYNC_ALPN)"] --> KEY{"key pinned for a pair?"}
    KEY -- no --> R1["close: code 1"]
    KEY -- yes --> ON{"channel on here?"}
    ON -- no --> R2["close: code 2"]
    ON -- yes --> MINE{"own connection to the peer<br/>made now or within 3 s,<br/>and own id smaller?"}
    MINE -- yes --> R3["close: code 3<br/>own connection is kept"]
    MINE -- no --> OLD{"a session runs<br/>with the peer?"}
    OLD -- "yes: the peer started again,<br/>or it won the crossing" --> REP["close the old session,<br/>wait until it releases"]
    OLD -- no --> HOLD
    REP --> HOLD["SpaceSync::hold_session<br/>claim the slot, run the session"]
    HOLD -- "slot taken" --> R4["close: code 4"]
```

## SpaceSync

`SpaceSync` (`sync/space_sync.rs`) is one object that the
`DeviceConnectionState` owns. It owns:

- the wake signal: a change, a frame received, or a session that comes up
  wakes it (`wake`, `wake_after`);
- the keeper: the task that runs a tick when woken. It starts with the app.
  On Android, the Bluetooth channel and the foreground service wait until the
  frontend has called in (`activity_ready`), because they need the Activity;
- the sessions: every live link goes to `hold_session`, which runs
  `run_session` on it;
- which channel a frame goes over: the first green channel in the pair's
  order, the primary first (`channel_order_for`).

Work for a device that has no green channel waits in the outbox. Nothing in
SpaceSync connects or retries: the channels connect by themselves, and a
session that comes up wakes the next tick.

## The objects and their files

| Object | File | Owns | Who calls it |
|---|---|---|---|
| `DeviceConnectionState` | `pairing/mod.rs` | the identity, the channels, the session slots, SpaceSync | the Tauri commands |
| `NetworkChannel` | `channel/network.rs` | the IP endpoint, the mDNS lookup, own connections in flight | the tick (`keep_serving`), setup (`connect_plain`) |
| `BluetoothChannel` | `channel/bluetooth/mod.rs`, `endpoint.rs` | the Bluetooth endpoint, the delivery schedule | the tick, setup, pairing |
| `Scanner` | `channel/bluetooth/scanner.rs` | the one Bluetooth scan, the devices heard, turns on the radio | `BluetoothChannel`, pairing |
| `Advertiser` | `channel/bluetooth/advertiser.rs` | what this device advertises, and if it does | `BluetoothChannel` |
| `BluetoothAdapter` | `channel/bluetooth/adapter.rs` | the adapter of this process (one per process) | the Bluetooth objects |
| `Gate` | `pairing/gate.rs` | the rules for who may connect | iroh's `Router` |
| `SpaceSync` | `sync/space_sync.rs` | when sync runs, the sessions, the channel order | the channels, the gate, the commands |
| `BluetoothCarrier` | `pairing/carrier.rs` | the links that Bluetooth pair requests came on | pairing |

## A DataLink and a session

A `DataLink` (`channel/mod.rs`) is one open connection to one device. It moves
bytes and nothing more. It has no device id, no quests and no outbox. Two
types implement it:

```rust
struct IrohDataLink {     // one QUIC stream on an iroh connection
    send, recv,
    peer_addr: Option<String>,
    peer_key: String,     // the key TLS proved
}

struct BleDataLink {      // a GATT characteristic, before pairing only
    channel: DatagramChannel,
    peer_addr: String,
}
```

A session (`sync/session.rs`, `run_session`) owns its link. The session slot
registry in `DeviceConnectionState` keeps a sender to the session's task, not
the link:

```rust
pub enum SessionCommand { Forward(PeerFrame), Close }
```

Thus there is no lock on each write, and two writers cannot mix the fragments
of two frames on Bluetooth. A session ends when its link ends.

## Frame, envelope, framing

- `PeerFrame`: one message. A value.
- `FrameEnvelope` (`{ v, enc, payload }`): the versioned wrapper around an
  encoded frame, so that encryption can be added later.
- Framing: where one blob ends on the byte pipe. On a QUIC stream this is a
  length prefix; the connecting side first writes an empty frame, so that the
  other side sees the stream (`STREAM_START`). On Bluetooth before pairing, it
  is the GATT fragments.

## Not done yet

- Bluetooth keeps high connection priority on each link. A held link should
  use low power while idle (step 0 measured about +7 mAh a day for a held
  low-power link). This needs a ble-gatt change.
- Android should reconnect from its background scan that the Bluetooth chip
  filters, not from Fini's own scan. This needs a ble-gatt change.
- A Bluetooth channel setup took about 60 s on hardware: Fini connects to
  old Bluetooth addresses of the phone that Android has rotated.
- SpaceSync finds what to send from the outbox, one event at a time. Issue
  #196 is about a change mark per device and space.
