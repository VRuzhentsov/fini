# historyGrouping

History deduplication utility consumed by [[HistoryView]].

## Behaviour

`sortHistoryItems` takes a flat array of resolved quests (`completed` or `abandoned`) and returns a sorted, deduplicated list:

- **Repeating quests** (those with a `repeat_rule`): the backend gives one row per closed day (ADR-0012); only the most recent one is kept. Recency is determined by `completed_at` when present, falling back to `updated_at`.
- **One-time quests**: passed through unchanged.
- All rows are sorted newest-first.

## Exports

| Export | Role |
|---|---|
| `sortHistoryItems` | Deduplicates and sorts history rows; returns `Quest[]` |
| `historyQuestTime` | Shared timestamp selector for History ordering |
