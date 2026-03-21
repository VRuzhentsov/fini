# QuestSeries

Template record for repeating quests. A series defines cadence and generation rules; actionable work happens in [[QuestOccurrence]] / [[Quest]].

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | uuid string | Stable series identifier |
| `space_id` | string | Parent [[Space]] id (`"1"`, `"2"`, `"3"`, or UUID); never null |
| `title` | string | Default title for generated occurrences |
| `description` | string \| null | Default description |
| `repeat_rule` | RepeatRule | Recurrence definition; see [[RepeatRule]] |
| `priority` | enum | Default priority for generated occurrences |
| `energy` | enum | Default energy for generated occurrences |
| `active` | boolean | Whether series continues generating occurrences |
| `created_at` | datetime | |
| `updated_at` | datetime | |

## Behavior

- Completing an occurrence should keep series active and prepare the next nearest open occurrence.
- Only the closest not-yet-resolved occurrence is surfaced in active quest lists by default.
- Historical occurrences remain visible in [[HistoryView]] and history endpoints.
