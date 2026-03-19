# Reminder

A scheduled notification linked to a [[Quest]]. Multiple reminders can be set per quest, each firing independently.

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | integer | Unique identifier |
| `quest_id` | integer | Parent quest |
| `type` | enum | `relative` or `absolute` — see [Types](#types) |
| `mm_offset` | integer \| null | Minutes before quest `due` + `due_time`; used when `type = relative` |
| `due` | datetime \| null | Exact fire time; used when `type = absolute` |

## Types

| Value | Behaviour |
|---|---|
| `relative` | Fires `mm_offset` minutes before the quest's `due` + `due_time`. Requires both to be set on the quest. Automatically adjusts when the quest's due date or time changes. |
| `absolute` | Fires at the exact `due` datetime stored on the reminder. Independent of the quest's due date. |

### Common relative presets

The UI offers quick-pick options for `mm_offset`:

| Label | mm_offset |
|---|---|
| 5 minutes before | 5 |
| 30 minutes before | 30 |
| 1 hour before | 60 |
| 1 day before | 1440 |

The user can also enter a custom value.

## Delivery

Notifications are delivered at the OS level — they fire regardless of whether the app is in the foreground, minimized, or closed.

| Platform | Channel |
|---|---|
| Android | System notification channel |
| Linux (Flatpak) | OS notification system |
| Windows | OS notification system |
| macOS | OS notification system |

## Content

| Element | Value |
|---|---|
| Title | Quest title |
| Body | Space name, if the quest is assigned to a space |

## Tap action

Tapping the notification brings the app to the foreground and navigates to the quest.

## Snooze

The notification offers snooze options: 10 minutes, 30 minutes, 1 hour. Snoozing schedules a new one-off `absolute` reminder at the selected offset and dismisses the current notification.

## Suppression

A reminder is not delivered if the quest has already been completed or abandoned by the time it fires.

## Repeating quests

When a repeating quest is completed and the next occurrence is scheduled, all `relative` reminders are automatically re-registered against the new due date. `Absolute` reminders are not carried over.
