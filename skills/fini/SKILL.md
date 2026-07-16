---
name: fini
description: "TRIGGER when user asks to create, update, list, or manage quests, spaces, reminders, or Focus state in Fini. Use the `fini-cli` foundation skill for binary preflight, CLI/app mode selection, JSON output decisions, and safe command sequencing before any action."
---

# Fini — Quest & Space CLI Workflow

You manage quests, spaces, reminders, and Focus state in the Fini app through the `fini` binary.

## Binary Preflight

At the start of every `fini` skill run, verify the global CLI command exists:

```bash
command -v fini
```

If `fini` is missing, broken, or not globally available, use `fini-setup` first. Do not run quest, space, reminder, or Focus commands until the installed binary passes `fini --help`.

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

### Nullable Quest Updates

For `fini quest update`, omit a field to preserve it, pass ordinary text (including an empty string) to store it, and pass literal `null` to clear nullable values such as `description`, `due`, `due_time`, and `repeat_rule`.

### Archive Export and Recovery Planning

Use structured JSON for archive workflows:

```bash
fini --json export --path /path/to/backup.zip --space-id <ID>
fini --json export --path /path/to/backup.zip --all-spaces
fini --json import --path /path/to/backup.zip --inspect
fini --json import --path /path/to/backup.zip --verify
fini --json import --path /path/to/backup.zip --dry-run
```

- `export` requires either one or more repeatable `--space-id` values or `--all-spaces`; never combine them.
- `import --inspect` validates and summarizes the archive without opening the local database.
- `import --verify` and `import --dry-run` preflight against the local database in read-only mode. `--dry-run` reports `ready_to_apply` and never applies recovery.
- For custom-space mappings, repeat `--map-space BACKUP_ID=create_new` or `--map-space BACKUP_ID=use_existing:LOCAL_ID`. Each backup ID may appear only once.
- Root import application and JSON migration persistence are still under implementation. Do not present inspection, verification, or dry-run as a completed recovery.

## Quick Start

```bash
# Recurring routine task
fini quest create --title "Pay rent" --repeat monthly

# Non-mutating archive inspection
fini import --path backup.zip --inspect --json
```

## Command Discovery

For arguments, accepted values, defaults, and workflows beyond these basic examples, use the built-in help surface:

```bash
fini --help
fini quest update --help
fini import --help
```

## Focus and App Entry Behavior

- `fini` with no args returns current Focus quest.
- `fini-app` launches the desktop GUI explicitly.
- Use `fini-cli` when the task is mainly about launching the app or validating the binary rather than managing quest domain state.

## Failure Handling

- Prefer fail-fast behavior for invalid IDs, invalid state transitions, or missing required arguments.
- Surface a short human explanation and the exact command the user can run next.
