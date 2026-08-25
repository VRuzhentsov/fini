# QuestList

Shared list container used by [[FocusView]] (active backlog section) and [[HistoryView]]. Renders one [[QuestListItem]] per quest; owns only the accordion (one row expanded at a time) and nothing about how a row looks or behaves.

## Props

| Prop | Type | Description |
|---|---|---|
| `quests` | `Quest[]` | Quest rows to display |

## Accordion

`expandedId` tracks which single quest (if any) is expanded. Each `QuestListItem` receives `:expanded="expandedId === quest.id"` and emits `toggle` (no payload); the list flips `expandedId` between that quest's id and `null`. This is the only state the list owns — everything else (row rendering, row actions, checklist scope, reminders) lives in `QuestListItem`.

## Dependencies

| Dep | Role |
|---|---|
| [[QuestListItem]] | Renders each row's collapsed/expanded state and behavior |
