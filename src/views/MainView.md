# MainView

Route: `/main`. Tab: Main. See [[App.md]].

## Concept

Focus screen — one quest at a time. The user sees only their current active quest and a quick-capture input. All quest management lives in [[QuestsView]]; completed and abandoned quests live in [[HistoryView]].

## Sections

### Active quest
The single hero quest. If none exists, a placeholder is shown. Renders [[ActiveQuestPanel]] with Complete and Abandon actions.

### Input
Always visible. Quickly captures a new quest title via [[NewQuestForm]].

