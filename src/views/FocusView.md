# FocusView

Route: `/main`. Tab: Focus. See [[App.md]].

## Concept

Focus-first workspace. FocusView shows the current Focus quest and also hosts active backlog management, so users can act without leaving the focus surface.

## Space filtering

All sections respect the active space from [[SpacePicker]]. When a specific space is selected, only quests with that `space_id` are shown. "All spaces" shows everything.

## Sections

### Focus quest panel

- Renders [[ActiveQuestPanel]] for the current computed Focus quest
- If no active quests exist, shows an empty-state placeholder
- Complete and Abandon actions are one-click

### Active backlog

- Shows all active quests below Focus quest
- Default ordering: overdue, then `order_rank`, then priority, then oldest `created_at`
- Includes expand/edit controls via [[QuestList]]
- Each quest has explicit "Set Focus" action (manual focus override)
- Drag-and-drop reorder is currently deferred in UI

### Quick capture

- Always-visible [[NewQuestForm]] for creating new quests

## Focus quest computation contract

- Computed by getter over persisted quest data/events
- Manual focus and restore triggers append events in [[FocusHistory]]
- Active reminder due timestamps are virtual Focus events: once `due + due_time` arrives, the reminder timestamp competes with manual Focus timestamps and the youngest valid timestamp wins
- Reminder preemption is temporary and unwinds to previous valid target
- If no active override exists, fallback is overdue > `order_rank` > priority > oldest `created_at`

See `specs/e2e/focus-reminder-preemption.md` for the open-app e2e contract.
