# HistoryView

Route: `/history`. Tab: History. See [[App.md]].

## Concept

Read-only list of completed and abandoned quests. No editing — only restore (make active) or delete via a context menu.

## Sections

### Quest list
Each row shows a status badge and the quest title.
- **Status badges** — `completed` (green), `abandoned` (amber)
- **Right-click / long-press** → opens a context menu at the cursor

### Context menu
| Action | Description |
|---|---|
| Make active | Sets `status = "active"`; quest reappears in [[MainView]] |
| Delete | Permanently removes the quest |

The menu is teleported to `<body>` and closes on any click outside.

## State

Uses [[quest.ts]] store. Loads on mount; `history` is a computed filter (`completed` or `abandoned`).

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | Fetch, update status, delete |
