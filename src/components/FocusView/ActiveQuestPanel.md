# ActiveQuestPanel

Primary card for the current Focus quest. Used in [[FocusView]].

## Props

| Prop | Type | Description |
|---|---|---|
| `quest` | `Quest` | The current Focus quest to display |

## Layout

Header row with the quest title on the left and action buttons on the right. Optional description below.

When `quest.focus_entry_count >= 2`, the card shows a quiet warm attention badge such as `Focus 3 times`. At `4+`, it adds `Keeps returning`. The copy is informational and must not imply failure.

## Actions

| Button | Behaviour |
|---|---|
| Complete | Sets `status = "completed"` via [[quest.ts]] |
| Abandon | Sets `status = "abandoned"` via [[quest.ts]] |

Both actions call `updateQuest` directly with one-click behavior (no confirmation dialog).

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest` for status changes |
