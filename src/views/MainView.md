# MainView

Route: `/main`. Tab: Main. See [[App.md]].

## Concept

Focus screen — one quest at a time. The user sees their current active quest and a quick-capture input. No history, no completed quests — those live in [[HistoryView]].

## Sections

### Active quest
Shows the first quest with `status = "active"`. If none exists, a placeholder is shown. Renders [[ActiveQuestPanel]] with Complete and Abandon actions.

### Input
Always visible. Captures a new quest title and creates it immediately via [[NewQuestForm]].

### Backlog
Shown only when more than one quest is active. Lists the other active quests (all active except the hero) via [[QuestList]], each with a "Make active" button to promote it to the hero slot.

## State

Uses [[quest.ts]] store. Loads all quests on mount; `activeQuest` and `backlog` are derived computed values — no separate fetch.

## Components

| Component | Purpose |
|---|---|
| [[ActiveQuestPanel]] | Hero quest with Complete / Abandon actions |
| [[NewQuestForm]] | Quick-capture input |
| [[QuestList]] | Backlog list with "Make active" per row |
