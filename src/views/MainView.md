# MainView

Main screen of the app. Route: `/quests`.

## Concept

There is always an active quest. The text input and microphone input are always visible and accessible — the user can capture a new quest or add to the current one at any time without navigating away.

## Layout

### Active quest
Always shown. Displays the current quest title and its steps with checkboxes. Includes complete and abandon actions.

### Input bar (always visible)
- **Text input** — type a quest description and submit
- **Microphone button** — start voice input, transcribe on-device, confirm steps

### History section
Listed below the active quest. Shows all past quests with status badges and a delete button.

## Components used
| Component | Purpose |
|---|---|
| `ActiveQuestPanel` | Current quest + steps |
| `NewQuestForm` | Text and voice input for new quests |
| `QuestList` | Quest history |
