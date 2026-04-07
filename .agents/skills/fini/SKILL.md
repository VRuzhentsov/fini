---
name: fini
description: "TRIGGER when user asks to create, update, list, or manage quests or spaces. Use the Fini CLI (`fini`) with a mandatory binary-access preflight before any action."
---

# Fini — Quest & Space CLI Workflow

You manage quests and spaces in the Fini app through the `fini` binary. Follow the rules below for every operation.

## Mandatory Preflight (Required Before Any Action)

Run these checks first:

1. `command -v fini`
2. `fini --help`

If either fails, stop immediately and return concrete remediation steps.
Do not perform read or write actions until preflight passes.

## Available CLI Commands

- `fini` or `fini focus get` — current Focus quest
- `fini quest ...` — quest list/get/create/update/complete/abandon/delete/history
- `fini space ...` — space list/create/update/delete
- `fini reminder ...` — reminder list/create/delete

Use human-readable output by default. Use `--json` when deterministic machine parsing is required.

## Rules

### Before Creating a Quest

1. **Always run `fini space list --json` first** to get available spaces and their IDs.
2. Determine which space fits the quest best based on its topic and space names.
3. Pass the matching `space_id` to `fini quest create`. If no space clearly fits, default to Personal (`"1"`).
4. If the user explicitly names a space, use that space's ID.

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

## Failure Handling

- Prefer fail-fast behavior for invalid IDs, invalid state transitions, or missing required arguments.
- Surface a short human explanation and the exact command the user can run next.
