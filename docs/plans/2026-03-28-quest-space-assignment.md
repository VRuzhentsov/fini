---
feature: Quest-Space Assignment UI
status: draft
---

# Quest-Space Assignment

Quests already have `space_id` in the DB/backend. This feature exposes space assignment in the UI.

## Components

### 1. SpacePicker (new component)

Top-row, right-aligned dropdown in the app menu bar. Uses DaisyUI dropdown.

- Lists all spaces + "All spaces" option
- Selected space = **default space** for new quests AND active filter for quest lists
- Selection persists to localStorage across restarts
- Default on first launch: "All spaces"

### 2. Quest Context Menu — "Move to space"

Right-click a quest row in QuestList → context menu with "Move to space" → submenu listing all spaces (excluding current).

- Selecting a space calls `updateQuest(id, { space_id })`
- Uses DaisyUI menu/submenu components
- Current space shown with checkmark or disabled

### 3. Quest List Filtering

- When a specific space is selected in SpacePicker: quest list filtered to that `space_id`
- When "All spaces" selected: all quests shown (optionally with space badge)
- Applies to both QuestsView and MainView (active quest + backlog)

## Behavior

| Action | Effect |
|--------|--------|
| Select space in SpacePicker | Filters quest list, sets default for new quests, persists to localStorage |
| Create quest | Uses selected space as `space_id` (falls back to "1" if "All spaces") |
| Right-click quest → Move to space → [Space] | Updates quest's `space_id` via backend |
| MainView | Active quest + backlog respect current space filter |

## No backend changes needed

- `create_quest` already accepts `space_id`
- `update_quest` already accepts `space_id`
- `get_quests` returns all quests; filtering done client-side

## UI library

All new UI uses DaisyUI components (dropdown, menu).
