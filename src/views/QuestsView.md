# QuestsView

Route: `/quests`. Tab: Quests. See [[App.md]].

## Concept

Full quest browser. Where [[MainView]] enforces single-quest focus, this view exposes all active quests at once — for planning, prioritisation, and management.

## Responsibilities

- List all active quests
- Support filtering (by space, priority, energy) and sorting
- Allow creating, editing, and reordering quests
- Promote a quest to the hero slot in [[MainView]] ("Make active")

## Out of scope

Completed and abandoned quests belong in [[HistoryView]].
