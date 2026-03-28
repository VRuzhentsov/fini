# MainView

Route: `/main`. Tab: Main. See [[App.md]].

## Concept

Focus-first workspace. Main shows the current Main quest and also hosts active backlog management, so users can act without leaving the focus surface.

## Space filtering

All sections respect the active space from [[SpacePicker]]. When a specific space is selected, only quests with that `space_id` are shown. "All spaces" shows everything.

## Sections

### Main quest panel

- Renders [[ActiveQuestPanel]] for the current computed Main quest
- If no active quests exist, shows an empty-state placeholder
- Complete and Abandon actions are one-click

### Active backlog

- Shows all active quests below Main quest
- Default ordering: overdue, then `order_rank`, then priority, then oldest `created_at`
- Includes expand/edit controls via [[QuestList]]
- Each quest has explicit "Set Main" action (manual focus override)
- Drag-and-drop reorder is currently deferred in UI

### Quick capture

- Always-visible [[NewQuestForm]] for creating new quests

## Main quest computation contract

- Computed by getter over persisted quest data/events
- Manual set main and reminder triggers use timestamps
- Reminder preemption is temporary and unwinds to previous valid target
- If no active override exists, fallback is overdue > `order_rank` > priority > oldest `created_at`
