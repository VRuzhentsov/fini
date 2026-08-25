# ReminderMenu

Bottom-sheet / popover opened by the Clock icon in [[QuestList]]. Lets the user set or clear `due`, `due_time`, and `repeat_rule` on a quest in one place.

## Trigger

Clock (⏱) icon button in the QuestList expanded card footer. Tapping it opens this menu for the quest being edited.

## Layout

```
┌──────────────────────────────────────┐
│ 🔍 Type a date…                      │  ← search field (visual only, not wired)
├──────────────────────────────────────┤
│  ★ Today    📅 Tomorrow              │  ← quick-pick chips
│  📅 Next week                        │
├──────────────────────────────────────┤
│  ◀  April 2026  ▶                    │  ← month-navigable calendar
│  Mo Tu We Th Fr Sa Su                 │
│  ...day grid (circular cells)...      │
├──────────────────────────────────────┤
│  ⏱ Time              09:00      ⌄    │  ← accordion: tap row to expand
├──────────────────────────────────────┤
│  ↻ Repeat            every week  ⏻   │  ← switch-driven accordion
├──────────────────────────────────────┤
│  [ Clear ]           [ Done ]        │
└──────────────────────────────────────┘
```

## Sections

### Search field

Text input with a placeholder ("Type a date…"). Visual only — not wired to any date-parsing logic. Reserved for a future freetext date search.

### Quick-pick chips

| Label | Sets `due` to |
|---|---|
| Today | today's date |
| Tomorrow | today + 1 day |
| Next week | today + 7 days |

Tapping a chip immediately highlights the selection; it is committed when Done is tapped.

### Calendar

Monday-first month grid with in-place navigation (prev/next chevrons + month/year label). Leading/trailing days from the adjacent months are shown, muted, and clickable — clicking one selects that date and navigates the calendar to its month. Today gets a ring outline; the selected day is filled with the primary color.

There is no separate "choose a date" row — the calendar is the only date-picking affordance.

### Time (accordion)

Tapping the row toggles a panel with boxed hour/minute steppers (increment/decrement buttons around an editable, keyboard-steppable number). The row shows the current value (`HH:MM` or "—") and a chevron that rotates when open. Opening for the first time (no `due_time` set yet) defaults to one hour from now. Closing clears `due_time`.

### Repeat (switch-driven accordion)

The row's pill switch is the only control that opens/closes the panel (the row itself isn't clickable). Turning it on reveals:

- An "Every [amount]" stepper (1–99).
- A Day / Week / Month / Year segmented unit control.
- A Mo–Su weekday circle picker, shown only when the unit is Day.

Turning the switch off hides the panel but does not discard the configured amount/unit/weekdays — turning it back on restores them. Saved as `repeat_rule: {"preset":"custom","interval":<amount>,"unit":<unit>,"days_of_week":[...]}` (the `days_of_week` key is omitted when no weekday is selected). When `days_of_week` is set, the amount counts weeks (every N weeks on those weekdays) regardless of the `unit` field — see [[RepeatRule]].

Opening the menu for a quest whose `repeat_rule` already holds one of the named presets (`daily`/`weekly`/`monthly`/`yearly`/`weekdays`/`weekends`) populates the amount/unit/weekday controls with the equivalent state.

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
| [[RepeatRule]] | `repeat_rule` JSON shape and recurrence semantics |
| [[QuestList]] | Parent — renders the trigger button |
