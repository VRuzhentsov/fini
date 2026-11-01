# QuestListItem

Renders a single quest's row inside [[QuestList]]: collapsed row or expanded [[QuestEditor]], plus that quest's own [[ReminderMenu]] and recurrence-scope prompt. Self-contained — reads its own store/space/context-menu state, the same pattern [[ActiveQuestPanel]] uses for the single always-expanded Focus quest.

## Props / events

| Name | Direction | Description |
|---|---|---|
| `quest` | prop | The quest to render |
| `expanded` | prop | Whether this row is the one currently expanded (owned by [[QuestList]]'s accordion) |
| `toggle` | emit | Requests this row's expanded state be flipped (collapsed-row click, or the editor's collapse action) |

## Row states

Rendering adapts per `quest.status`.

### Active quests

#### Collapsed

Checkbox, due/repeat badge, title, checklist badge, non-medium energy/priority badges, space badge.

#### Expanded

- Header: checkbox, editable title, Focus indicator/action, collapse
- Body: editable description plus accessible Energy (Small / Medium / Large) and Priority (Low / Medium / High) controls
- Footer:
  - Left: due/time/repeat summary (opens [[ReminderMenu]])
  - Right: attachment (future), labels (future), priority, more menu

#### Active row actions

| Action | Behavior |
|---|---|
| Complete | Sets `status = completed` |
| Set Focus | Appends manual focus event in [[FocusHistory]] |
| Abandon | Sets `status = abandoned` |
| Delete | Permanent delete with confirmation |

Energy is a quest effort estimate (Small / Medium / Large) and Priority is urgency (Low / Medium / High); both default to Medium and are hidden from the collapsed row at the default level so the glance surface stays quiet. Priority participates in ordering as high > medium > low after overdue and order-rank precedence.

### History rows

#### Collapsed

Checked checkbox (green completed / amber abandoned), timestamp badge, struck-through title.

#### Expanded

- Header: checked checkbox, title, timestamp, collapse
- Body: read-only description
- Footer menu: Make active, Delete

Deleting from history is permanent and requires confirmation.

## Checklist recurrence scope (issue #128)

Adding/removing/renaming a checklist item on a repeating quest opens [[RecurrenceScopeSheet]] to choose "This day" vs "All days" before applying the edit (ADR-0012 D8). "This day" edits that day's copy of the checklist; "All days" edits the quest's template, and the backend merges the edit into every open day copy. Ticking an item never asks: a tick is always that day's. In History, "Make active" on a day of a repeating quest opens only that day again.

## Context menu

Right-click the row (or its collapsed/expanded surface) to open [[ContextMenu]] via `useContextMenu()` with "Move to space" submenu.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest`, `deleteQuest`, checklist mutations |
| [[QuestEditor]] | Expanded row body |
| [[ReminderMenu]] | Due date / time / repeat controls |
| [[RecurrenceScopeSheet]] | Checklist edit-scope prompt for recurring quests |
| [[ContextMenu]] | Right-click menus (via `useContextMenu()`) |
| [[buildQuestMenu]] | Standard context-menu items |
| [[QuestList]] | Parent — owns the accordion's `expanded` state |
