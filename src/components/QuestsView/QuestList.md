# QuestList

Shared list UI used by [[MainView]] (active backlog section) and [[HistoryView]]. Rendering adapts per `quest.status`.

## Props

| Prop | Type | Description |
|---|---|---|
| `quests` | `Quest[]` | Quest rows to display |

## Row states

Each row supports collapsed and expanded states. One row can be expanded at a time.

## Active quests

### Collapsed

Checkbox + title.

### Expanded

- Header: checkbox, editable title, Main-focus indicator/action, collapse
- Body: editable description
- Footer:
  - Left: due/time/repeat summary (opens [[ReminderMenu]])
  - Right: attachment (future), labels (future), priority, more menu

### Active row actions

| Action | Behavior |
|---|---|
| Complete | Sets `status = completed` |
| Set Main | Writes manual focus timestamp (`set_main_at`) |
| Abandon | Sets `status = abandoned` |
| Delete | Permanent delete with confirmation |

Energy is stored but hidden from MVP controls.

## History rows

### Collapsed

Checked checkbox (green completed / amber abandoned), timestamp badge, struck-through title.

### Expanded

- Header: checked checkbox, title, timestamp, collapse
- Body: read-only description
- Footer menu: Make active, Delete

Deleting from history is permanent and requires confirmation.

## Context menu

Right-click a quest row to open [[ContextMenu]] via `useContextMenu()` with "Move to space" submenu.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest`, `deleteQuest` |
| [[ReminderMenu]] | Due date / time / repeat controls |
| [[ContextMenu]] | Right-click "Move to space" (via `useContextMenu()`) |
