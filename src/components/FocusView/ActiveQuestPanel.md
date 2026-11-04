# ActiveQuestPanel

Primary card for the current Focus quest. Used in [[FocusView]].

## Props

| Prop | Type | Description |
|---|---|---|
| `quest` | `Quest` | The current Focus quest to display |

## Layout

Header row with the quest title on the left and action buttons on the right. Optional description below.

## Actions

| Button | Behaviour |
|---|---|
| Complete | Tap: sets `status = "completed"` via [[quest.ts]]; for a repeating quest this closes its shown (oldest open) day. Hold (repeating quest only): a menu with Complete and "Complete through today", which closes every open day dated today or before (ADR-0012 D7) |
| Abandon | Sets `status = "abandoned"` via [[quest.ts]] |

Both actions call `updateQuest` directly with one-click behavior (no confirmation dialog).

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest` for status changes |
