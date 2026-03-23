# MCP Contract Baseline Notes (Issue #3)

This note documents the initial MCP contract baseline for series/occurrence alignment and structured outputs.

## Summary

- MCP tools now return structured JSON in `structured_content` (no human-formatted text output).
- Quest outputs include occurrence fields derived from `repeat_rule` for MVP.
- `get_active_quest` mirrors the event-driven Main resolver used in the app.

## QuestRecord schema

| Field | Type | Notes |
|---|---|---|
| `id` | string | Quest identifier (UUID string) |
| `series_id` | string \| null | Derived for repeating quests; see rules below |
| `occurrence_id` | string \| null | Derived occurrence id; see rules below |
| `period_key` | string \| null | Derived UTC date key; see rules below |
| `space_id` | string | Space id (`"1"`, `"2"`, `"3"`, or UUID) |
| `title` | string | Quest title |
| `description` | string \| null | Optional description |
| `status` | string | `active`, `completed`, `abandoned` |
| `priority` | integer | Priority value |
| `energy` | string | `low`, `medium`, `high` |
| `due` | string \| null | Date `YYYY-MM-DD` |
| `due_time` | string \| null | Time `HH:MM` or `HH:MM:SS` |
| `due_at_utc` | string \| null | Derived UTC deadline `YYYY-MM-DDTHH:MM:SSZ` |
| `repeat_rule` | string \| null | Raw stored rule (JSON string or preset name) |
| `set_main_at` | string \| null | Manual Main override timestamp |
| `reminder_triggered_at` | string \| null | Reminder override timestamp |
| `order_rank` | float | Signed range `-100..100` |
| `completed_at` | string \| null | Completion timestamp |
| `created_at` | string | Creation timestamp |
| `updated_at` | string | Update timestamp |

## Occurrence derivation (MVP)

If `repeat_rule` is set and not `"none"`:

- `series_id = quest.id`
- `period_key` uses `due` (if present), otherwise the UTC date of `created_at`
- `occurrence_id = "{series_id}:{period_key}"`

If `repeat_rule` is null or `"none"`, all occurrence fields are `null`.

## SpaceRecord schema

| Field | Type | Notes |
|---|---|---|
| `id` | string | Space id (`"1"`, `"2"`, `"3"`, or UUID) |
| `name` | string | Space name |
| `item_order` | integer | Ordering index |
| `created_at` | string | Creation timestamp |

## Tool output changes

- `list_quests` -> `QuestRecord[]` (active by default, optional `status` filter)
- `get_quest` -> `QuestRecord`
- `create_quest` -> `QuestRecord`
- `update_quest` -> `QuestRecord`
- `complete_quest` -> `QuestRecord`
- `abandon_quest` -> `QuestRecord`
- `delete_quest` -> `{ "deleted": true, "id": "..." }`
- `list_history` -> `QuestRecord[]` (completed + abandoned)
- `get_active_quest` -> `QuestRecord` or `null`
- `list_spaces` -> `SpaceRecord[]`
- `create_space` -> `SpaceRecord`
- `update_space` -> `SpaceRecord`
- `delete_space` -> `{ "deleted": true, "id": "..." }`

## Client update checklist

- Read structured JSON from `structured_content`.
- Treat all ids as opaque strings (no numeric casts).
- Parse `repeat_rule` only if needed; it is returned as a raw string.
- Use occurrence fields for repeating quest semantics in clients.
