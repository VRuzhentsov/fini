# QuestList

Shared list UI used by [[FocusView]] (active backlog section) and [[HistoryView]]. Rendering adapts per `quest.status`.

## Props

| Prop | Type | Description |
|---|---|---|
| `quests` | `Quest[]` | Quest rows to display |
| `groupChildrenById` | `Record<string, Quest[]>` (optional) | When set, quests whose id appears as a key are treated as group representatives. Value is the sorted children array (newest first). Used by [[HistoryView]] for series grouping. |

## Row states

Each row supports collapsed and expanded states. One row can be expanded at a time.

## Active quests

### Collapsed

Checkbox + title.

### Expanded

- Header: checkbox, editable title, Focus indicator/action, collapse
- Body: editable description
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

Energy is stored but hidden from MVP controls.

## History rows

### Collapsed

Checked checkbox (green completed / amber abandoned), timestamp badge, struck-through title.

### Expanded

- Header: checked checkbox, title, timestamp, collapse
- Body: read-only description
- Footer menu: Make active, Delete

Deleting from history is permanent and requires confirmation.

## Group rows (History series grouping)

When `groupChildrenById` is provided and a quest id appears as a key, that row renders as a group representative.

### Collapsed

Same layout as a standalone history row plus a `ChevronRightIcon` (`data-testid="quest-row-group-expander"`) at the trailing edge. The chevron rotates 90° when expanded.

**Status badge** reflects the group as a whole:
- Single-status group → normal `Completed` or `Abandoned` pill.
- Mixed group (both statuses present) → neutral `Mixed N / M` pill (`N` completed, `M` abandoned).

**Check glyph** restores the latest resolved occurrence (`children[0]`) to active.

### Expanded

A nested `<ul data-testid="quest-row-group-children">` of child rows renders via `v-if="expanded"` (children fully unmount on collapse). Each child shows its own status badge, timestamp, title, and space badge. The child check glyph restores that individual occurrence. No delete action on children.

The group header surface remains visible while expanded (`v-if="expandedId !== quest.id || getGroupChildren(quest.id)"`), so the chevron is always reachable for collapse.

### Group context menu

Right-clicking the representative row opens a two-item menu:

| Action | Behavior |
|---|---|
| Make active latest | `updateQuest(children[0].id, { status: "active" })` |
| Delete series | `window.confirm(…)` → `deleteQuestSeries(series_id)` — deletes every past and future occurrence and the `quest_series` row |

Non-group rows retain the standard `buildQuestMenu` context menu.

## Context menu

Right-click a quest row to open [[ContextMenu]] via `useContextMenu()` with "Move to space" submenu.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest`, `deleteQuest`, `deleteQuestSeries` |
| [[ReminderMenu]] | Due date / time / repeat controls |
| [[ContextMenu]] | Right-click menus (via `useContextMenu()`) |
| [[buildQuestMenu]] | Standard context-menu items for non-group rows |
| [[historyGrouping]] | Produces `groupChildrenById` consumed by this component |
