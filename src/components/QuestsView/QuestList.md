# QuestList

Shared list container used by [[FocusView]] (active backlog section) and [[HistoryView]]. Renders one [[QuestListItem]] per quest; owns only the accordion (one row expanded at a time) and nothing about how a row looks or behaves.

## Props

| Prop | Type | Description |
|---|---|---|
| `quests` | `Quest[]` | Quest rows to display |
| `reorderable` | `boolean` | When true, a person can move the rows by drag-and-drop. [[FocusView]] and the Quests view set it; [[HistoryView]] does not |

## Accordion

`expandedId` tracks which single quest (if any) is expanded. Each `QuestListItem` receives `:expanded="expandedId === quest.id"` and emits `toggle` (no payload); the list flips `expandedId` between that quest's id and `null`. Apart from a drag in progress, this is the only state the list owns — everything else (row rendering, row actions, checklist scope, reminders) lives in `QuestListItem`.

## Drag-and-drop

With `reorderable`, a collapsed row moves with native pointer events:

- Mouse: press the button on a row and move it more than 8 px. A click with no move still opens the row.
- Finger or pen: hold the row for 350 ms, then move it. A move before that time scrolls the list. A long press with no move opens the context menu (`contextmenu`) and does not move the row.
- While a row moves, it takes the place of the row under the pointer; the page does not scroll.
- At the drop, the list calls `store.moveQuest(id, { before: nextRow.id })` (or `{ after: previousRow.id }` at the end): one `move_quest` command. The store shows the new order at once and goes back to the old order if the command fails. The backend writes only the moved quest, except for a rebalance (glossary "Rank").
- An expanded row does not move, so text selection in the editor still works.

## Dependencies

| Dep | Role |
|---|---|
| [[QuestListItem]] | Renders each row's collapsed/expanded state and behavior |
