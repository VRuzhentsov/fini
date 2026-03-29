# HistoryView

Route: `/history`. Tab: History. See [[App.md]].

## Concept

Read-only list of completed and abandoned quests/occurrences. No inline editing - only restore (make active) or permanent delete via context menu.

## Sections

### Quest list
Each row shows a status badge and the quest title.
- **Status badges** — `completed` (green), `abandoned` (amber)
- **Right-click / long-press** → opens a context menu at the cursor

### Context menu
| Action | Description |
|---|---|
| Make active | Sets `status = "active"`; quest reappears in [[FocusView]] focus surface |
| Delete | Permanently removes the quest after confirmation |

The menu is teleported to `<body>` and closes on any click outside.

## State

Uses [[quest.ts]] store. Loads on mount; `history` is a computed filter (`completed` or `abandoned`).

For repeating quests, history is occurrence-level (each occurrence appears as its own entry).

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | Fetch, update status, delete |
