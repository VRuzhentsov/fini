---
name: fini
description: "TRIGGER when user asks to create, update, list, or manage quests or spaces. Guides correct use of Fini MCP tools (quest lifecycle, space assignment, reminders)."
---

# Fini — Quest & Space MCP Workflow

You manage quests and spaces in the Fini app via MCP tools. Follow the rules below for every quest/space operation.

## Available MCP Tools

- `list_spaces` — list all spaces (id, name, order)
- `create_space` / `update_space` / `delete_space` — manage spaces
- `list_quests` — list quests by status and optional space filter
- `get_quest` / `get_active_quest` — fetch single quest or current Main
- `create_quest` — create a new quest
- `update_quest` — update quest fields
- `complete_quest` / `abandon_quest` / `delete_quest` — lifecycle transitions
- `list_reminders` / `create_reminder` / `delete_reminder` — reminder management

## Rules

### Before Creating a Quest

1. **Always call `list_spaces` first** to get available spaces and their IDs.
2. Determine which space fits the quest best based on its topic and space names.
3. Pass the matching `space_id` to `create_quest`. If no space clearly fits, default to Personal (`"1"`).
4. If the user explicitly names a space, use that space's ID.

### Built-in Spaces

| ID  | Default Name |
|-----|-------------|
| `1` | Personal    |
| `2` | Family      |
| `3` | Work        |

Users may rename built-ins or create custom spaces — always fetch fresh data via `list_spaces`.

### Quest Defaults

- `space_id` defaults to `"1"` (Personal) if not provided
- `status` starts as `active`
- `energy` defaults to `medium`
- `priority` defaults to `1`

### Due Dates

- `due` — ISO date string (e.g. `2026-03-28`)
- `due_time` — HH:MM format (optional)
- Both are optional; omit if user doesn't mention a deadline

### Repeating Quests

- Set `repeat_rule` for recurring quests (e.g. `daily`, `weekly`, `monthly`)
- Repeating quests track `series_id` and `period_key` automatically
