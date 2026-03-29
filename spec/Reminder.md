# Reminder

Scheduled notification linked to a [[Quest]] / [[QuestOccurrence]]. Multiple reminders can exist per quest.

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | uuid string | Reminder identifier |
| `quest_id` | uuid string | Parent quest/occurrence |
| `type` | enum | `relative` or `absolute` |
| `mm_offset` | integer \| null | Minutes before `due_at_utc` for `relative` reminders |
| `due_at_utc` | datetime \| null | Exact trigger time for `absolute` reminders |
| `created_at` | datetime | |

## Types

| Value | Behavior |
|---|---|
| `relative` | Triggers `mm_offset` minutes before quest `due_at_utc` |
| `absolute` | Triggers exactly at reminder `due_at_utc` |

## Common relative presets

| Label | mm_offset |
|---|---|
| 5 minutes before | 5 |
| 30 minutes before | 30 |
| 1 hour before | 60 |
| 1 day before | 1440 |

## Delivery

- Uses OS-level notifications on Android/Linux/Windows/macOS
- Works with foreground, background, minimized, or closed app

## Trigger effects

- If the target quest is already `completed` or `abandoned`, trigger is suppressed.
- If the target quest is active, reminder trigger can preempt current Focus.
- Reminder preemption is temporary; Focus returns to previous valid target after reminder quest resolves.
- Reminder-triggered focus writes a `trigger = reminder` event to [[FocusHistory]].

## Snooze

- Snooze options: 10m, 30m, 1h
- Snooze creates a one-off `absolute` reminder for the current occurrence
- Snooze does not alter repeat cadence or series schedule

## Permissions

If notification permission is denied, reminder metadata remains editable and a subtle visible warning is shown in UI.
