# QuestsView

Legacy route: `/quests`.

## Concept

Transitional active-quest browser kept during migration. Final MVP navigation removes the Quests tab and moves active backlog management into [[FocusView]].

## Responsibilities

- Keep parity while Focus backlog UI is being consolidated
- Reuse [[QuestList]] behavior for active quest editing
- Reuse [[NewQuestForm]] for Quest creation so the legacy route supports the same Space, reminder, and Description composer behavior as [[FocusView]]


## Out of scope

Completed and abandoned quests belong in [[HistoryView]].
