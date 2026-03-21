# QuestOccurrence

Concrete dated instance produced from a [[QuestSeries]]. In UI and MCP, this is represented as a normal actionable quest record ([[Quest]]).

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | string | Deterministic occurrence id derived from `series_id + period_key` |
| `series_id` | uuid string | Parent [[QuestSeries]] |
| `period_key` | string | Deterministic period key (UTC boundary based) |
| `due_at_utc` | datetime \| null | Canonical deadline for this occurrence |
| `status` | enum | `active`, `completed`, `abandoned` |
| `completed_at` | datetime \| null | Completion timestamp |
| `completed_by` | string \| null | Actor label (hostname) in shared spaces |
| `created_at` | datetime | |
| `updated_at` | datetime | |

## Rules

- Deterministic occurrence `id` prevents duplicates when offline devices generate the same period occurrence.
- In shared spaces, one completion resolves the occurrence for all paired devices.
- Completion/abandonment suppresses pending reminders for that occurrence.
