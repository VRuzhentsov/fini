# DeviceConnection

LAN discovery, presence, and pairing control-plane behavior for Fini.

## Authority split

- `spec/DeviceConnection.md` owns discovery/presence/pairing UX + control-plane behavior.
- `spec/SpaceSync.md` owns space mapping + data replication behavior.
- `spec/Network.md` owns transport-level contracts shared by both services.

## Settings information architecture

Routes:

- `/settings`
- `/settings/add-device`
- `/settings/device/:id`

`/settings` section order:

1. Spaces
2. Devices
3. Voice Model

Devices section behavior:

- Show paired devices only.
- Device row opens `/settings/device/:id`.
- `Add device` row is always last and opens `/settings/add-device`.

## Device identity and local record

Minimum identity fields:

- `device_id` (UUID, immutable)
- `hostname`

Minimum paired-device record fields:

- `peer_device_id`
- `display_name`
- `paired_at`
- `last_seen_at`
- `pair_state`

Display identity:

- Primary label: `display_name`
- Disambiguation: short UUID suffix

## Presence model

- While app process is alive, every device emits heartbeat on LAN.
- Normal heartbeat interval: 60s.
- Offline threshold: 2 missed heartbeats (120s).
- `last_seen_at` is derived from latest heartbeat/discovery packet.

## Add-device mode

Add-device mode is active only in `/settings/add-device`.

- Pairing requests are processed only when both peers are in add-device mode.
- Discovery cadence in add-device mode: every 5s.
- Candidate list rules:
  - newest seen first
  - dedupe by `device_id`
  - hide already paired devices
- Leaving add-device view cancels pending requests immediately.

## Pairing flow

- Pairing passcode is mandatory (6 digits).
- Sender role:
  - first click timestamp wins
  - tie-breaker: lower `device_id`
- Receiver sees incoming request sheet.
- Sender sees code only after receiver accepts.
- Wrong code policy: 3 attempts then 60s cooldown.
- Pending request timeout: 60s.
- Pairing survives restart until unpair.

## Transport scope

- UDP is used for connection control-plane only (discovery/presence/pairing).
- Discovery beacon advertises fixed `space_sync` websocket port for data-plane.
- UDP payload changes do not carry replicated quest/space/reminder/focus domain data.

## Security policy (connection layer)

- Pairing passcode is required.
- Successful pairing produces pair-auth material used by `space_sync` websocket handshake.
- Transport encryption for data-plane is deferred to follow-up phase.

## Public command naming

- Hard cut-over to `device_connection_*` command naming.
- No backward command aliases.
