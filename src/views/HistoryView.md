# HistoryView

Route: `/history`. Tab: History. See [[App.md]].

## Concept

Read-only list of completed and abandoned quests. No inline editing — only restore (make active) or delete via context menu.

## Sections

### Quest list

Rows show a status badge and the quest title, sorted newest-first.

- **Status badges** — `completed` (green), `abandoned` (neutral)
- **Right-click / long-press** → opens a context menu at the cursor

For repeating quests, only the most recently resolved occurrence appears; earlier occurrences are omitted.

### Context menu

| Action | Description |
|---|---|
| Make active | Sets `status = "active"`; quest reappears in [[FocusView]] |
| Move to space | Moves the quest to another space |
| Delete | Permanently removes the quest |

## State

Uses [[quest.ts]] store. Loads on mount; `history` is a computed filter (`completed` or `abandoned`). [[historyGrouping]] deduplicates series rows and sorts, then the result is passed directly to `<QuestList>`.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | Fetch, update status, delete |
| [[historyGrouping]] | Deduplicates series rows to latest occurrence |
| [[QuestList]] | Shared list component |
