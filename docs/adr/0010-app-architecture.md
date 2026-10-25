# 0010 — The architecture of the app

## Status

Accepted. Written during the grilling session of 2026-10-25. The
communication part of the app is built to it.

## Context

This ADR fixes the architecture of the whole app: which kinds of parts it
has and how they connect. The communication code is the first part that
moves to it.

The communication code grew around one object, `DeviceConnectionState`,
that every Tauri command receives. It holds the identity, the channels, the
session slots and SpaceSync. Pairing and channel setup are modules of
functions, and the `Gate` holds the rules for who may connect. The names
do not tell what each part is: `DeviceConnectionState` reads as an enum,
and "Gate" has more than one meaning.

Designs compared during the session: Syncthing (layers), KDE Connect (one
object per device), libp2p (swarm, transports, behaviours), iroh (one
handler per protocol), Briar, Home Assistant (event bus, registries,
config flows), vertical slices, and commands with events (CQRS-light).

## Decisions

**D1 — Five patterns.** The app is built from RPC (Tauri commands),
services, repositories, PubSub (an event bus) and DI, each used through an
interface. They are defined in
[`code-design.md`](../architecture/code-design.md) and the
[glossary](../glossary.md).

**D2 — The role is in the type name.** A type ends with its role
(`DeviceService`, `DeviceRepository`); a model is one noun (`Device`); an
interface is the general noun, and each implementation adds the word that
makes it different ([`naming.md`](../naming.md)).

**D3 — Role folders.** `src-tauri/src/` has `commands/`, `models/`,
`repositories/` and `services/` next to each other. The folder is the
role. Communication moves into them.

**D4 — One `DeviceService`, no flow objects.** Pairing (Add Device),
channel setup and unlink are methods of one `DeviceService`, so that other
parts can call them. Home Assistant's flow objects were considered and
rejected: the steps are shared, and a flow per dialog splits them.

**D5 — No hub object.** `DeviceConnectionState` goes. The app start builds
each service with its dependencies (DI) and registers each one with
Tauri's `app.manage()`. A command asks only for the services it uses
(`State<'_, DeviceService>`).

**D6 — The service that owns a protocol accepts its connections** (the
Router that hands them over is in D15).
`DeviceService` implements iroh's `ProtocolHandler` for `fini/peer/1`
(pairing, `AddChannel`, `IdentifyDevice`), and `SpaceSyncService` for
`fini/sync/4`. Each asks `DeviceRepository` which paired device has the
key that TLS proved. The `Gate` object goes. iroh-blobs and iroh-gossip
use the same form.

**D7 — Flat service files.** A service is one file in `services/`, named
`<name>.service.rs`. Its helper objects stay in the same file (the
Bluetooth scanner and advertiser, the SpaceSync session). A folder is
considered only when a service needs more than one file.

**D8 — One event bus with one `Event` enum.** The `EventBus` is a
`tokio::sync::broadcast` of one `Event` enum that lists every event of the
app. A subscriber matches the events that it needs. A subscriber that gets
`Lagged` reads the current state again. Syncthing's `lib/events` and Home
Assistant's bus use this form. Typed topics (Bevy) and one `subscribe()`
for each service (iroh watchers) were rejected: no single list of events,
more code, and more dependencies between services.

**D9 — The UI hears the bus.** One task subscribes to the `EventBus` and
emits each `Event` as a Tauri event. The Pinia stores listen and update
their state; the polling of commands goes. Commands stay for actions and
for the first read of a screen. The `Event` enum is thus part of the
contract between the backend and the frontend.

**D10 — Channel names.** The interface stays `ChannelService`. The
implementations are `BluetoothChannelService` and `NetworkChannelService`.
`Channel` stays the model of a channel row (on or off, primary).

**D11 — One repository for each model.** `DeviceRepository` (`Device`,
today `PairedDevice`), `ChannelRepository` (`Channel`),
`SpaceMappingRepository` (`SpaceMapping`, today `PairSpaceMapping`) and
`OutboxRepository` (the outbox entries and their acks). This is the form
of the existing quest and reminder repositories. A service that needs a
device and its channels asks two repositories.

**D12 — An event is a change, with the new state.** Each event tells
that one thing changed and carries its new state. The services and the UI
hear the same events. The list of events belongs to the code, not to this
ADR: it changes with the implementation. A subscriber that missed events is correct again after
the next event for the same thing.

**D13 — In PR #197.** The refactor is done on the branch
`fix/ble-add-device-scan`, together with the Bluetooth and Network work
that it carries.

**D14 — A channel owns its connections; SpaceSync owns its sessions.**
Each `ChannelService` holds its live, TLS-proven connections and publishes
when one comes up or goes down. Green means that the channel holds a live
connection to the device. `SpaceSyncService` hears that, opens its session
on the connection, and owns the sessions. A channel never calls SpaceSync:
the dependency goes one way.

**D15 — Dependencies go one way; the identity is loaded first.** The app
start builds the parts in this order:

```
app start (composition root)
1. Identity             model: id, name, key; read from the settings rows
                          device.id, device.name, device.secret_key
2. ChannelService × 2   get the identity as values, and the event bus
3. DeviceService,       get the channel services, the repositories and
   SpaceSyncService       the event bus
4. iroh Router          one for each channel endpoint, built last:
                          fini/peer/1 → DeviceService,
                          fini/sync/4 → SpaceSyncService
```

A part uses only the parts above it. A channel service knows nothing about
devices or pairing. `DeviceService` holds the paired devices; a rename of
this device goes through it and is published on the bus. Syncthing, KDE
Connect, libp2p and iroh apps (sendme, dumbpipe) use this order. Keeping
the identity inside `DeviceService` was rejected: the channel services need
the key to start, and `DeviceService` needs the channel services, which
makes a loop.

**D16 — One Pinia store for each backend service.** Each backend service
has one store, named `<name>.store.ts` (`device.store.ts`,
`channel.store.ts`, `space-sync.store.ts`). A store calls the commands of
its service and applies the events of its service. Components use stores
only. A reader goes from a screen to a store to a service.

**D17 — Commands are grouped by service.** The commands of a service are
in one file, `<name>.commands.rs`, and their names start with the name of
the service. File names carry the role, as [`naming.md`](../naming.md)
tells.

**D18 — The bus is a service, an event is a model.** The bus is
`EventBusService` in `services/event-bus.service.rs`; the `Event` enum is
in `models/event.rs`. No new role folder.
