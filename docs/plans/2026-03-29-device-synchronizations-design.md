# Device Synchronizations Design (Issue #4)

Date: 2026-03-29
Status: Implementation-ready spec lock
Scope: naming split + sync architecture + Focus model

## Problem split

- `device_connection`: discovery/presence/pairing control-plane
- `space_sync`: mapped-space replication data-plane

No UDP redesign in this ticket.

## Naming lock

- Hard cut-over naming, no aliases.
- `device_sync` naming is removed.
- Public surfaces rename to:
  - `device_connection_*`
  - `space_sync_*`
- Product term rename: `Main` -> `Focus`.

## Specs lock

- Keep and update:
  - `spec/Network.md`
  - `spec/Quest.md`
  - `spec/Space.md`
  - `spec/Reminder.md`
  - `src/views/SettingsView.md`
  - `src/views/AddDeviceView.md`
  - `src/views/DeviceView.md`
- New:
  - `spec/DeviceConnection.md`
  - `spec/SpaceSync.md`
  - `spec/FocusHistory.md`
- Delete:
  - `spec/DeviceSync.md`

## Transport lock

- UDP for control-plane only.
- WebSocket for sync data-plane only.
- WS endpoint uses fixed port and is advertised in discovery beacon.
- One canonical WS session per pair (deterministic dialer).

## Sync lock

- Durable outbox + ACK replay required.
- Generic sync event envelope required.
- Include `event_id` and `correlation_id`.
- Conflict order:
  1. `updated_at`
  2. `origin_device_id`
  3. `event_id`
- Relay/fan-out enabled across pair graph.
- Tombstones required, retention 30 days.

## Mapping lock

- Mapping is pair-level symmetric.
- Mapping key is `space_id`.
- Enabling mapping triggers immediate bootstrap sync.
- Missing mapped spaces auto-create on peer with same `space_id`.

## Domain lock

Replicated domain for mapped spaces:

- spaces
- quests
- series + occurrences
- reminders
- focus history (filtered)

## Focus lock

- Focus is owner-scoped metadata.
- Move focus metadata from `quests` row fields to `focus_history` entity.
- Focus can target quests in any space.
- Focus sync allowed only for peers that map `Personal` (`"1"`) between each other.
- Focus sync replicates only entries tied to mapped spaces.

## Reminder lock

- Reminder metadata replicates.
- Each mapped peer schedules/fires local OS notification.

## Security lock

- Pair-auth required for WS sync session.
- Transport encryption deferred to follow-up ticket.

## Persistence lock

- Pairing + mapping source-of-truth moves to backend SQLite.
- Frontend localStorage runtime pairing state is dropped (re-pair flow accepted).

## MCP lock

- No new sync MCP operations in this ticket.
- Rename only existing naming surfaces/contracts where applicable.

## Test gate

- Unit + integration tests for queue/replay/conflict/tombstones/fan-out/focus filters.
- Manual 2-device smoke for pairing, mapping, bootstrap, offline reconnect convergence.

## Unresolved questions

- none
