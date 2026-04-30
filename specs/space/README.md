# Space

## Scope

Local space model, built-in spaces, and Settings-based space management.

## Responsibilities

- Define built-in spaces such as `Personal`
- Support create, rename, and delete for custom spaces
- Keep stable `space_id` identity for sync and mapping
- Provide the local source of truth for space lists used by settings and sync

## Behavior

- Built-in spaces `1`, `2`, and `3` always exist
- Built-in spaces cannot be deleted
- Custom spaces can be created, renamed, and deleted
- Sync features rely on stable `space_id`, not display name matching

## Primary UI Surface

- `src/views/SettingsView.vue`

## Related Feature

- `specs/space-sync/README.md`

## Wiki Links

- `~/projects/fini-wiki/pages/concepts/Space.md` when present
