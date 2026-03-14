# QuestList

List of quests used in [[QuestsView]].

## Props

| Prop | Type | Description |
|---|---|---|
| `quests` | `Quest[]` | Quests to display |

## Row layout

Each row contains:
1. **Checkbox** — marks the quest as completed
2. **Metadata pill** — shown only when `due` or `repeat` is set; displays due date and repeat summary (e.g. `Mar 17, ↻ every week (tu,we)`)
3. **Title**

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest` |
