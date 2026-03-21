# RepeatRule

Recurrence rule attached to a [[QuestSeries]] (or directly to a [[Quest]] that participates in a series). When an occurrence is completed, the next nearest occurrence is scheduled automatically.

## Presets

| Value | Meaning |
|---|---|
| `daily` | Every day |
| `weekdays` | Monday–Friday |
| `weekends` | Saturday–Sunday |
| `weekly` | Same day each week |
| `monthly` | Same date each month |
| `yearly` | Same date each year |
| `none` | No recurrence |

## Custom rule

Selected via the "Custom" option in the repeat picker.

| Field | Type | Description |
|---|---|---|
| `interval` | integer | Repeat every N units (e.g. `2` = every 2 weeks) |
| `unit` | enum | `day` \| `week` \| `month` \| `year` |
| `days_of_week` | day[] \| null | Active days; applicable when `unit = week` |
| `end` | enum | `never` \| `on_date` \| `after_n` |
| `end_date` | date \| null | Used when `end = on_date` |
| `end_after` | integer \| null | Number of occurrences; used when `end = after_n` |

## End conditions

| Value | Behaviour |
|---|---|
| `never` | Repeats indefinitely |
| `on_date` | Last occurrence is on or before `end_date` |
| `after_n` | Stops after `end_after` completions |

## Occurrence generation

- Recurrence evaluation uses UTC period boundaries.
- Generated occurrences are represented by [[QuestOccurrence]] and surfaced as actionable [[Quest]] records.
- Active lists should show only the closest unresolved occurrence per series; earlier resolved occurrences stay in history.
