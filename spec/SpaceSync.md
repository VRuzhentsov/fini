# SpaceSync

Per-space data synchronization between paired devices.

## Scope

- Replicates selected spaces between paired devices.
- Includes domain data for mapped spaces:
  - spaces metadata
  - quests
  - quest series and occurrences
  - reminders
  - focus history (owner-scoped, filtered by mapped spaces)

## Service boundary

- `device_connection` handles discovery/presence/pairing.
- `space_sync` handles mapping + replication only.

`space_sync` does not replace or redesign UDP pairing/discovery logic.

## Mapping model

- Mapping is pair-level and symmetric.
- A mapping update on one peer is replicated and becomes effective on both peers.
- Mapping unit is `space_id` (id-based, never name-based).

## Bootstrap behavior

- When a mapping is enabled for a space, run immediate bootstrap sync.
- Bootstrap includes existing records for that space, not future-only changes.
- If mapped space does not exist on peer, auto-create it with the same `space_id`.

## Runtime lifecycle

- Sync runs while app process is alive.
- If app is closed, no background daemon/service is required in this phase.
- Reconnect performs catch-up from durable queue.

## Transport and session model

- Data-plane uses websocket.
- Peer endpoint uses fixed websocket port advertised by `device_connection` beacon.
- Exactly one canonical websocket session per pair:
  - deterministic dialer rule picks a single initiator (lower `device_id` dials)
- Session requires pair-auth handshake before event exchange.

## Replication model

- Durable outbox + ACK replay.
- Every local mutation becomes a sync event.
- Event envelope is generic across entities.

Minimum event envelope fields:

- `event_id` (UUID)
- `correlation_id` (UUID)
- `origin_device_id`
- `entity_type`
- `entity_id`
- `space_id`
- `op_type` (`upsert`/`delete`)
- `payload` (nullable for tombstone deletes)
- `updated_at` (UTC)
- `created_at` (UTC)

## Quest synchronization between spaces

### Event scope

- Quest sync scope is always the quest `space_id` at mutation time.
- A peer receives a quest event only when that event `space_id` is currently mapped for the pair.

### Cross-space moves

- Moving a quest between spaces is a first-class sync transition and must converge on both peers.
- Expected behavior by move type:
  - mapped -> mapped: quest stays as one logical record (`id` unchanged) and updates `space_id` on peer.
  - mapped -> unmapped: peer removes quest copy from mapped dataset (no stale copy remains visible).
  - unmapped -> mapped: peer receives quest and shows it in the newly mapped space.

### Mapping toggles

- Enabling mapping for a space runs bootstrap and includes existing quests from that space.
- Disabling mapping for a space stops future replication for that space.
- Disabling mapping does not require immediate destructive cleanup of already-synced historical rows in this phase.

### Quest lifecycle operations

- Create, update, complete/abandon/restore, and delete must replicate for mapped spaces.
- Deletes replicate via tombstones and must not resurrect after reconnect.

## Convergence and conflict policy

- Required outcome: automatic eventual convergence after reconnect.
- Conflict policy:
  1. newer `updated_at` wins
  2. if equal, lexicographically lower `origin_device_id` wins
  3. if still equal, lexicographically lower `event_id` wins
- Event dedupe key is `event_id`.

## Fan-out topology

- Relay/fan-out is enabled.
- If A is paired with B and C for mapped space X, events from B can be relayed by A to C.
- Relay must preserve `event_id` for loop prevention/dedupe.

## Delete semantics

- Deletes replicate via tombstones.
- No resurrection after reconnect.
- Tombstone retention: 30 days, then cleanup/compaction.

## Reminder semantics

- Reminder records replicate for mapped spaces.
- Each mapped device schedules and fires its own local OS notification.

## Focus semantics

- Product term is `Focus` (replaces `Main`).
- Focus state is owner-scoped and stored in [[FocusHistory]], not in shared quest rows.
- Focus can target quests from any space.
- Focus history replication filters:
  - replicate only events whose target quest belongs to a mapped space
  - owner-cluster is implicit: peers that map `Personal` (`space_id = "1"`) share focus history

## Security policy (sync layer)

- Pair-auth is mandatory for websocket sync session.
- Data-plane transport encryption is deferred to follow-up phase.

## MCP scope for this phase

- No new `space_sync` MCP operations in this phase.
- Naming surfaces follow hard cut-over conventions.
