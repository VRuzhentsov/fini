# ActiveQuestPanel

Hero card for the currently active quest. Used in [[MainView]].

## Props

| Prop | Type | Description |
|---|---|---|
| `quest` | `Quest` | The active quest to display |

## Layout

Header row with the quest title on the left and action buttons on the right. Optional description below.

## Actions

| Button | Behaviour |
|---|---|
| Complete | Sets `status = "completed"` via [[quest.ts]] |
| Abandon | Sets `status = "abandoned"` via [[quest.ts]] |

Both actions call `updateQuest` directly — no confirmation dialog.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest` for status changes |
