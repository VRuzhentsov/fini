# ReminderMenu

Bottom-sheet / popover opened by the Clock icon in [[QuestList]]. Lets the user set or clear `due`, `due_time`, and `repeat_rule` on a quest in one place.

## Trigger

Clock (⏱) icon button in the QuestList expanded card footer. Tapping it opens this menu for the quest being edited.

## Layout

```
┌──────────────────────────────────────┐
│ 🔍 Type a date…                      │  ← date search / freetext
├──────────────────────────────────────┤
│  ★ Today    📅 Tomorrow              │  ← quick-pick chips
│  📅 Next week                        │
├──────────────────────────────────────┤
│  📅 Choose a date               >    │  ← opens native date picker
├──────────────────────────────────────┤
│  ↻  Repeat                      >    │  ← opens RepeatRule sub-sheet
├──────────────────────────────────────┤
│  ⏱ Time                         +    │  ← toggles time picker row
├──────────────────────────────────────┤
│  [ Clear ]           [ Done ]        │
└──────────────────────────────────────┘
```

## Sections

### Date search

Freetext field. Parses natural language dates (e.g. "friday", "next monday", "march 20"). On match, previews the resolved date. On submit (Enter or Done), applies the date.

### Quick-pick chips

| Label | Sets `due` to |
|---|---|
| Today | today's date |
| Tomorrow | today + 1 day |
| Next week | start of next week (Monday) |

Tapping a chip immediately highlights the selection; it is committed when Done is tapped.

### Choose a date

Opens the native `<input type="date">` picker. Selection previews in the chip row.

### Repeat (`>` chevron)

Opens [[RepeatRule]] sub-sheet. Current repeat summary shown inline if set (e.g. "every week").

### Time (`+` button)

Reveals a time picker row (`<input type="time">`). Sets `due_time`. The `+` toggles to `×` to clear the time.

## Actions

| Button | Behaviour |
|---|---|
| **Clear** | Clears `due`, `due_time`, and `repeat_rule`. Closes the menu. |
| **Done** | Saves `due`, `due_time`, and `repeat_rule`. On Android, the first reminder save that needs OS delivery requests notification permission before closing. |

Tapping outside the sheet or pressing Escape closes without saving.

## Props / events

| Name | Direction | Description |
|---|---|---|
| `quest` | prop | The quest being edited |
| `close` | emit | Menu requests to close (no save) |
| `save` | emit | `{ due, due_time, repeat_rule }` — user confirmed changes |

## Dependencies

| Dep | Role |
|---|---|
| [[Quest]] | `due`, `due_time`, `repeat_rule` fields |
| [[RepeatRule]] | Sub-sheet for recurrence |
| [[QuestList]] | Parent — renders the trigger button |
