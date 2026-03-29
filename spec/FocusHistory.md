# FocusHistory

Owner-scoped focus timeline used to compute current Focus quest.

## Purpose

- Replaces quest-row focus fields (`set_main_at`, `reminder_triggered_at`).
- Keeps focus metadata separate from shared quest content.
- Supports synchronization rules that differ from quest data replication.

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | uuid string | Focus event id |
| `device_id` | uuid string | Origin device that wrote the focus event |
| `quest_id` | uuid string | Target quest id |
| `space_id` | string | Target quest space id (denormalized for filtering) |
| `trigger` | enum | `manual`, `reminder`, `restore`, `system` |
| `created_at` | datetime | Event timestamp (UTC) |

## Semantics

- Focus is computed from focus-history events + current quest states.
- Latest valid focus event wins.
- If latest target is no longer active, resolver falls back to next valid event.
- If no valid focus event exists, fallback order from [[Quest]] applies.

## Replication

- Focus history is owner-scoped.
- Replicate only when owner-cluster rule matches:
  - owner-cluster is implicit through `Personal` mapping (`space_id = "1"`).
- Replicate only entries whose `space_id` is currently mapped for the pair.

## Restore behavior

- Restoring a quest from history appends a new `FocusHistory` event with trigger `restore`.
