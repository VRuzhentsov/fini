# Network & Local Sync

Fini is local-first and accountless. LAN sync is introduced in **MVP.1**.

## Authority split

- [[DeviceConnection]] owns discovery/presence/pairing control-plane behavior.
- [[SpaceSync]] owns mapping and data replication behavior.
- `spec/Network.md` owns transport-level contracts shared by both.

## Goals

- Zero-cloud architecture (no central account service)
- Explicit pairing before data sharing
- Per-space synchronization selection
- Automatic convergence after offline reconnect

## Entry points

- GUI app
- MCP server
- Headless runtime

All entry points operate on the same local dataset.

## Transport split

- Control-plane: UDP
  - discovery
  - presence heartbeat
  - pairing handshake
- Data-plane: websocket
  - space synchronization events
  - ACK/replay traffic

Discovery beacons advertise fixed websocket port for `space_sync`.

## Pairing baseline

- Pairing requires a 6-digit passcode.
- Pairing survives restarts until unpair.
- Presence heartbeat interval: 60s.
- `last_seen_at` is derived from latest heartbeat/discovery receipt.

## Session model

- One canonical websocket session per paired device pair.
- Deterministic initiator rule selects the dialer (lower `device_id` dials).
- Websocket session requires pair-auth handshake before data exchange.

## Replication model

- Push-on-change + reconnect catch-up.
- Durable local outbox and ACK replay.
- Queue survives restart/crash.
- Event dedupe by `event_id`.
- Fan-out relay between connected peers is allowed.

## Conflict and convergence policy

Required outcome: automatic eventual convergence.

Conflict order:

1. newer `updated_at` wins
2. tie-break: lexicographically lower `origin_device_id` wins
3. final tie-break: lexicographically lower `event_id` wins

## Delete semantics

- Deletes are replicated as tombstones.
- No resurrection after reconnect.
- Tombstones are retained for 30 days, then cleaned up.

## Space identity policy

- Space identity is id-based, not name-based.
- Built-ins use reserved ids: `"1"`, `"2"`, `"3"`.
- Custom spaces use UUID ids.
- Missing mapped spaces are auto-created on peers with the same `space_id`.

## Shared repeating behavior

- Repeating quests use series + occurrences.
- Deterministic occurrence identity: `series_id + period_key`.
- `period_key` uses UTC period boundaries.
- Completion of shared occurrence cancels pending reminders for that occurrence on all mapped peers.

## Focus synchronization

- Product term is `Focus`.
- Focus events are owner-scoped in [[FocusHistory]], not in shared quest rows.
- Focus history sync is allowed only for owner-cluster peers (implicit via mapped `Personal` space `"1"`).
- Focus events replicate only when target quest belongs to a mapped space.

## Security

- LAN sharing is off by default.
- Pairing passcode is mandatory.
- Pair-auth is mandatory for websocket sync sessions.
- Data-plane transport encryption is deferred to follow-up phase.
- At-rest encryption is post-MVP work.
