Frontend state for [[Quest]]. Single source of truth for quest data in the UI.

## Actions

| Action | Description |
|---|---|
| `fetchQuests()` | Load all quests from the backend |
| `createQuest(input)` | Create a new quest; prepends it to `quests` |
| `updateQuest(id, input)` | Update a quest; replaces it in `quests` |
| `deleteQuest(id)` | Delete a quest; removes it from `quests` |

## Notes

- Components never call the backend directly — always go through this store.
- For the domain model see [[Quest]].
- Main quest selection uses persisted timestamps (`set_main_at`, reminder-trigger metadata) plus fallback rules.
