# QuestList

Shared list UI used by [[FocusView]] (active backlog section) and [[HistoryView]]. Rendering adapts per `quest.status`.

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

- Header: checkbox, editable title, Focus indicator/action, collapse
- Body: editable description plus accessible Energy (Small / Medium / Large) and Priority (Low / Medium / High) controls
- Footer:
  - Left: due/time/repeat summary (opens [[ReminderMenu]])
  - Right: attachment (future), labels (future), priority, more menu

### Active row actions

| Action | Behavior |
|---|---|
| Complete | Sets `status = completed` |
| Set Focus | Appends manual focus event in [[FocusHistory]] |
| Abandon | Sets `status = abandoned` |
| Delete | Permanent delete with confirmation |

Energy is a quest effort estimate (Small / Medium / Large) and Priority is urgency (Low / Medium / High); both default to Medium. Priority participates in ordering as high > medium > low after overdue and order-rank precedence.

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
| [[ContextMenu]] | Right-click menus (via `useContextMenu()`) |
| [[buildQuestMenu]] | Standard context-menu items |
