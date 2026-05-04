---
name: fini
description: "TRIGGER when user asks to create, update, list, or manage quests, spaces, reminders, or Focus state in Fini. Use the `fini-cli` foundation skill for binary preflight, CLI/app mode selection, JSON output decisions, and safe command sequencing before any action."
---

# Fini — Quest & Space CLI Workflow

You manage quests, spaces, reminders, and Focus state in the Fini app through the `fini` binary.

## Shared CLI Foundation

Use `fini-cli` for shared mechanics before any operation:

- mandatory binary preflight
- CLI mode vs app launch mode
- human-readable output vs `--json`
- safe command sequencing
- generic failure handling

Do not duplicate those checks here. This skill defines Fini domain behavior on top of `fini-cli`.

## Rules

### Before Creating a Quest

1. Use `fini-cli` preflight first.
2. Run `fini space list --json` to get available spaces and their IDs.
3. Determine which space fits the quest best based on its topic and space names.
4. Pass the matching `space_id` to `fini quest create`. If no space clearly fits, default to Personal (`"1"`).
5. If the user explicitly names a space, use that space's ID.

### Built-in Spaces

| ID  | Default Name |
|-----|-------------|
| `1` | Personal    |
| `2` | Family      |
| `3` | Work        |

Users may rename built-ins or create custom spaces. Always fetch fresh data via `fini space list`.

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

## Focus and App Entry Behavior

- `fini` with no args returns current Focus quest.
- `fini app` launches GUI explicitly.
- Use `fini-cli` when the task is mainly about launching the app or validating the binary rather than managing quest domain state.

## Failure Handling

- Prefer fail-fast behavior for invalid IDs, invalid state transitions, or missing required arguments.
- Surface a short human explanation and the exact command the user can run next.
