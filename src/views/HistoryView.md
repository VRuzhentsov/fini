# HistoryView

Route: `/history`. Tab: History. See [[App.md]].

## Concept

Read-only list of completed and abandoned quests/occurrences. No inline editing — only restore (make active) or permanent series delete via context menu.

## Sections

### Quest list

Standalone rows show a status badge and the quest title.
- **Status badges** — `completed` (green), `abandoned` (neutral)
- **Right-click / long-press** → opens a context menu at the cursor

Repeating quest occurrences are grouped when at least two resolved rows share the same `series_id`. Single resolved occurrences remain standalone.

### Grouped series row

Rendered by [[QuestList]] using the `groupChildrenById` prop — no dedicated component. The group row reuses the same collapsed-row markup as active and standalone history rows; the presence of a `groupChildrenById` entry drives the group affordance.

- Header shows the representative (latest child) status, timestamp, title, and Space badge.
- A `ChevronRightIcon` at the trailing edge signals expand/collapse (rotates 90° when open).
- Status badge is `Mixed N / M` when children have both completed and abandoned statuses; otherwise the uniform status label.
- Click row body or chevron to expand/collapse.
- Children render via `v-if="expanded"` on the group row; collapsing fully unmounts them (lightweight; no need to preserve child state).
- Children are sorted by `completed_at ?? updated_at` descending (newest first).

### Context menu

Standalone row menu:

| Action | Description |
|---|---|
| Make active | Sets `status = "active"`; quest reappears in [[FocusView]] |
| Delete | Permanently removes the quest after confirmation |

Grouped series header menu:

| Action | Description |
|---|---|
| Make active latest | Restores `children[0]` (latest occurrence) only (`status = "active"`) |
| Delete series | Confirms via `window.confirm`, then deletes every past and future occurrence and the `quest_series` row |

The menu is teleported to `<body>` and closes on any click outside.

## State

Uses [[quest.ts]] store. Loads on mount; `history` is a computed filter (`completed` or `abandoned`). [[historyGrouping]] converts those rows into `{ rows, groupChildrenById }` which is passed directly to `<QuestList>`.

For repeating quests, the backend still returns occurrence-level rows; grouping is History-only frontend presentation.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | Fetch, update status, delete, delete series |
| [[historyGrouping]] | History-only grouping; returns `{ rows, groupChildrenById }` |
| [[QuestList]] | Shared list component; receives `groupChildrenById` for series affordance |
