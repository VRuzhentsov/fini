# MCP ID Migration Notes (Issue #1)

This note documents MCP breaking changes introduced by the identity migration.

## Why

Fini moved from integer ids to stable string ids for cross-device compatibility:

- Quest `id`: `i64` -> UUID string
- Space `id`: `i64` -> string (`"1"`, `"2"`, `"3"`, or UUID)
- Quest `space_id`: nullable integer -> non-null string (default `"1"`)

## Tool input changes

### All tools using `id`

- Before: `id: number`
- After: `id: string`

Affected tools:

- `get_quest`
- `update_quest`
- `complete_quest`
- `abandon_quest`
- `delete_quest`
- `update_space`
- `delete_space`

### `list_quests`

- Before: `space_id?: number`
- After: `space_id?: string`

### `create_quest`

- Before: `space_id?: number`
- After: `space_id?: string`
- Behavior: omitted `space_id` now defaults to `"1"` (Personal)

### `update_quest`

- Before: `space_id?: number | null`
- After: `space_id?: string`
- `null` is no longer valid for `space_id`

## Data compatibility notes

- Existing databases are migrated forward automatically.
- Legacy integer ids are not preserved as public ids after migration; clients must treat ids as opaque.
- Built-in spaces are represented by reserved ids:
  - `"1"` Personal
  - `"2"` Family
  - `"3"` Work

## Client update checklist

- Parse all MCP ids as strings
- Stop numeric casts/comparisons for ids
- Update any stored references to treat ids as opaque string keys
- For space filtering, pass string ids (e.g. `"1"`)
