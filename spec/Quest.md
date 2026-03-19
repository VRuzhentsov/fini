# Quest

Core domain model. A quest is a single unit of intention — something the user commits to doing.

## Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `id` | integer | — | Unique identifier |
| `space_id` | integer \| null | null | Parent [[Space]], or null (unassigned) |
| `title` | string | — | Short label; the primary identity of the quest |
| `description` | string \| null | null | Optional longer note |
| `status` | enum | `active` | See [Status](#status) |
| `priority` | enum | `none` | See [Priority](#priority) |
| `energy` | enum | `medium` | See [Energy](#energy) |
| `pinned` | boolean | false | Pinned quests sort above others |
| `due` | date \| null | null | Due date |
| `due_time` | time \| null | null | Optional time component of the due date; required for `relative` [[Reminder]]s to compute their fire time |
| `repeat` | RepeatRule \| null | null | Recurrence rule; see [Repeat](#repeat) |
| `completed_at` | datetime \| null | null | Set when status transitions to `completed` |
| `created_at` | datetime | — | |
| `updated_at` | datetime | — | |

## State vs status

A quest is either **active** (in progress — the ongoing state) or has reached a terminal **status**: `completed` or `abandoned`. Active is not a status outcome; it is the default state of a quest while it is being worked on.

| Value | Kind | Meaning |
|---|---|---|
| `active` | state | In play — visible in [[MainView]] and [[QuestsView]] |
| `completed` | status | Done — moves to [[HistoryView]] |
| `abandoned` | status | Dropped — moves to [[HistoryView]] |

A quest in History can be restored to the active state. When restored, it is automatically pinned — it surfaces at the top of the active list, since it was previously in progress.

## Priority

| Value | Meaning |
|---|---|
| `none` | No urgency assigned |
| `low` | Nice to have |
| `medium` | Should do soon |
| `urgent` | Blocking or time-critical |

## Energy

How much mental or physical effort the quest requires. Used to match quests to the user's current capacity.

| Value | Meaning |
|---|---|
| `low` | Routine or mechanical; doable when tired |
| `medium` | Normal focus required (default) |
| `high` | Deep work; requires full attention |

## Repeat

See [[RepeatRule]].
