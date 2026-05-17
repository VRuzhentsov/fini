# historyGrouping

Pure grouping helpers for [[HistoryView]].

## Behaviour

- Input is the History subset of [[quest.ts]] rows: completed or abandoned quests.
- Rows with a non-null `series_id` are grouped only when that series has at least two resolved occurrences in History.
- Non-series rows and single resolved occurrences pass through unchanged.
- Group children sort by `completed_at ?? updated_at` descending, then `id` for stability.
- Group sort key is the latest child timestamp, so groups slot into the same newest-first History order as standalone rows.

## Return type

`groupHistoryItems` returns `{ rows: Quest[], groupChildrenById: Record<string, Quest[]> }`.

- `rows` — ordered list of display rows; each is either a standalone resolved quest or the representative (newest child) of a grouped series.
- `groupChildrenById` — map from representative quest id to its sorted children. A quest id present in this map signals [[QuestList]] to render the group affordance (chevron, mixed-status pill, group context menu).

## Exports

| Export | Role |
|---|---|
| `groupHistoryItems` | Converts quest rows to `{ rows, groupChildrenById }` |
| `historyQuestTime` | Shared timestamp selector for History ordering |
