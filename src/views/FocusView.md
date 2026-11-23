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
- Complete is one tap; on a repeating quest, holding Complete opens a menu with "Complete through today". Abandon is a hold

### Active backlog

- Shows all active quests below Focus quest
- Ordering: overdue (due date before today), then the rank key (`order_rank`, glossary "Rank"), then Priority (`high > medium > low`), then oldest `created_at`, then id. The backend (`Quest::list_order`), `fini quest list` and the store (`activeOrder`) use the same order
- Includes expand/edit controls via [[QuestList]]
- Each quest has explicit "Set Focus" action (manual focus override)
- Drag-and-drop reorder: a person moves a row to a new place ([[QuestList]], "Drag-and-drop"). An overdue quest stays in the overdue part at the top

### Quick capture

- Always-visible [[NewQuestForm]] for creating new quests

## Focus quest computation contract

- Computed by getter over persisted quest data/events
- Manual focus and restore triggers append events in [[FocusHistory]]
- Active reminder due timestamps are virtual Focus events: once `due + due_time` arrives, the reminder timestamp competes with manual Focus timestamps and the youngest valid timestamp wins
- Reminder preemption is temporary and unwinds to previous valid target
- A quest comes to Focus at its `due + due_time` (09:00 when there is no time); before that moment it is not on Focus. For a repeating quest this is the moment of its shown day: a daily meeting at 09:00 comes to Focus each day at 09:00
- If no active override exists, fallback is the first quest in the backlog order: overdue > rank key > Priority (`high > medium > low`) > oldest `created_at` > id

See `specs/e2e/focus-reminder-preemption.md` for the open-app e2e contract.
