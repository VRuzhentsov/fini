# DeviceSync

Device pairing, presence, and local sync behavior for Fini.

## Authority split

- `spec/Network.md` owns transport-level sync model (discovery baseline, queue/replay, conflict policy).
- `spec/DeviceSync.md` owns product-facing device behavior and settings UX flow.

## Settings information architecture

Routes:

- `/settings`
- `/settings/add-device`
- `/settings/device/:id`

`/settings` shows sections in this order:

1. Spaces (existing inline management)
2. Devices (inline `DeviceList`)
3. Voice Model

Devices section behavior:

- `DeviceList` shows paired devices only.
- Device row opens `/settings/device/:id`.
- `Add device` row is always last and opens `/settings/add-device`.

## Device identity and record

Minimum local record fields:

- `device_id` (UUID, immutable)
- `display_name` (default hostname)
- `paired_at`
- `last_seen_at`
- `pair_state`

Display identity:

- Primary label: `display_name`
- Disambiguation: short UUID suffix

Display-name editing is deferred for now.

## Presence model

- While app is open, every device emits a presence heartbeat on LAN (all routes/modes).
- Presence heartbeat interval (normal mode): 60s.
- `last_seen_at` is updated from the most recent presence heartbeat/discovery packet for that device.
- Online indicator:
  - green: seen recently
  - gray: offline
- Offline threshold: 2 missed heartbeats (120s total).

## Add-device mode

Add-device mode is active only in `/settings/add-device`.

- Pairing requests are processed only when both devices are in add-device mode.
- Discovery in add-device mode scans every 5s.
- Discovery list includes only peers currently in add-device mode.
- Discovery deduplicates by `device_id` and updates freshness timestamps.
- Ordering: newest seen first.
- Already paired devices are hidden in add-device list.
- Leaving add-device view cancels pending requests immediately.

## Pairing flow

- Pairing passcode is mandatory (6 digits).
- Sender/receiver selection:
  - first click timestamp wins sender role
  - tie-breaker: lower `device_id` wins
- Receiver behavior: incoming pairing request sheet is shown.
- Code reveal timing: sender sees code only after receiver accepts.
- Wrong code policy: 3 attempts per remote device, then 60s cooldown.
- Pending request timeout: 60s if ignored.
- Code expiry: no expiry while add-device view remains open.
- Successful pairing persists `device_id` and pairing metadata.

## Device detail view

`/settings/device/:id` currently supports:

- Connection status display (green/gray)
- Read-only mapping placeholder (`Mapped spaces: TBD`)
- `Unpair` action with required confirmation

Unpair semantics:

- Device is removed from `DeviceList` immediately.
- Existing local synced data is retained.
- Future sync with that device stops.

## Sync scope and constraints

- Platform parity is required: no behavior differences by platform.
- Queue durability is required (offline queue survives restarts/crashes).
- Queue schema is deferred.
- Mapping semantics are intentionally `TBD` and will be defined in a separate design session.

## Security phase policy

- MVP.1: passcode-gated pairing is required.
- Transport encryption is deferred beyond MVP.1.
- Later phase: encrypted transport becomes mandatory.
