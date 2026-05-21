# Backup Import/Export

Issue: #28

## Goal

Implement user-controlled portable backup import/export for Fini quests and spaces.

Backups are local files. Version 1 does not include cloud backup, scheduled backup, archive encryption, password protection, or an exact clone of every runtime table.

## File Format

Backup files are `.zip` archives named by default:

```text
fini-backup-YYYY-MM-DD.zip
```

The archive must contain exactly:

```text
manifest.json
fini-backup.sqlite
```

`manifest.json` records:

```json
{
  "format": "fini-backup",
  "version": 1,
  "app_version": "0.1.27",
  "exported_at": "2026-05-19T12:00:00Z",
  "domains": ["spaces", "quest_series", "quests"],
  "spaces": [{ "id": "1", "name": "Personal" }],
  "counts": { "spaces": 1, "quest_series": 0, "quests": 0 }
}
```

Import rejects non-zip files, raw SQLite files, JSON backup files, archives missing required files, archives with extra files, unsupported manifest versions, malformed backup SQLite databases, and databases missing required backup tables.

## Export

Export includes:

- Selected spaces.
- All quests in selected spaces, including active, completed, and abandoned.
- `quest_series` rows whose `space_id` is selected.

Export excludes:

- settings
- device identity
- paired devices
- space sync mappings
- sync outbox, acks, seen rows, and tombstones
- reminders
- series reminder templates
- notification snoozes
- focus history

Settings export flow:

1. User clicks `Export backup`.
2. Show a space checklist dialog.
3. No spaces are checked by default.
4. `Export` is disabled until at least one space is selected.
5. User chooses a save location through the native save/create-document picker.
6. Backend writes the zip.
7. Show `Backup exported`.

CLI export requires one or more `--space-id` values or explicit `--all-spaces`.

## Import

Settings import flow:

1. User clicks `Import backup`.
2. Native document picker opens and accepts `.zip` files.
3. Backend validates the zip and manifest before any write.
4. Backend preflights imported spaces, quests, and quest series.
5. Built-in spaces are hidden from mapping and automatically use matching IDs.
6. Custom backup spaces with existing local IDs use that ID automatically.
7. Missing custom backup spaces are resolved one at a time.
8. User chooses `Create new` or selects an existing local space to map.
9. After space mapping, item conflicts are resolved.
10. Import applies changes in one transaction.
11. Spaces, quests, and focus refresh in place; user stays on Settings.
12. Show `Backup imported`.

CLI import does no space mapping UI. It imports as-is by IDs, fails on conflicts by default, and `--force` uses backup versions.

## Space Mapping

Built-in space IDs are `1`, `2`, and `3`.

- Built-in spaces map to matching built-in IDs and are hidden from mapping UI.
- Incoming custom spaces whose IDs already exist locally map to that same ID automatically.
- Missing custom spaces require `create_new` or `use_existing`.
- `create_new` creates the local space using the backup space ID.
- `use_existing` assigns imported quests and quest series from that backup space to the selected local space ID.

## MergeConflictDialog

`MergeConflictDialog` is a reusable global dialog for one-at-a-time item conflicts. Backup is its first consumer.

User-facing labels:

- `Local` means current app data.
- `Backup` means incoming backup file data.

Layout:

```text
[Conflict title]                                      3/10

Local                         Backup
--------------------------    --------------------------
Current local summary         Incoming backup summary

[Copy]                        [Copy]
[Show]                        [Show]

[Use local]                   [Use backup]
```

Rules:

- Counter appears in the top-right corner in short `3/10` format.
- Counter numerator is the number of conflicts with selected resolutions.
- `Use local` and `Use backup` are the lowest buttons in their columns.
- `Copy` copies that column's visible/details content.
- `Show` expands full details for that column.
- `Use local` keeps current app data.
- `Use backup` applies the incoming backup version.
- There is no `Skip` button; `Use local` covers that behavior.
- Apply is disabled until every conflict has a selected resolution.
- Selected choice is visibly highlighted.

## Relations

Use the term `relations` for normal data relationships.

Relevant relations:

- A quest belongs to a space.
- A repeating quest belongs to a quest series.

The importer must not write quests or quest series in a state that breaks these relations.

## Acceptance Criteria

- Settings has a Backup section with `Export backup` and `Import backup`.
- Settings export uses a native save/create-document flow.
- Settings import uses a native open-document flow restricted to `.zip`.
- Android Settings import/export uses document picker/create-document style flows.
- Export checklist starts with no spaces selected.
- Export button is disabled until at least one space is selected.
- Export writes a `.zip` containing exactly `manifest.json` and `fini-backup.sqlite`.
- Export includes only selected spaces, their quests, and selected-space quest series.
- Import rejects any file that is not the v1 backup zip format.
- Import maps custom incoming spaces one at a time using the device-sync-style mapping pattern.
- Built-in spaces are auto-mapped and hidden from mapping.
- Existing local custom space IDs are reused without mapping.
- Missing custom space IDs require create-new or map-to-existing choice.
- Quest and quest-series conflicts use the generic `MergeConflictDialog`.
- CLI export requires `--space-id` or `--all-spaces`.
- CLI import fails on conflicts by default.
- CLI import `--force` uses backup versions for conflicts.
- Import applies changes in one transaction.
- After Settings import succeeds, app state refreshes and user stays on Settings.
