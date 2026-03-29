# Fini MVP Baseline (Locked 2026-03-21)

This document records the design decisions locked during the product interview and serves as the implementation baseline.

> Terminology and sync architecture updates were later introduced. See `docs/plans/2026-03-29-device-synchronizations-design.md` for the current `Focus` naming and `device_connection`/`space_sync` split.

## Scope by phase

### MVP

- Platforms: Linux, Windows, Android with functional parity
- Navigation: `Main`, `History`, `Settings`
- Active backlog management lives in `Main` (no dedicated `Quests` tab)
- Included in MVP:
  - Core quest lifecycle (create, edit, complete, abandon, restore, delete)
  - Reminder metadata + OS reminder delivery
  - Repeating quest `series + occurrences`
  - Main quest computation rules
  - MCP support (MVP-critical)
- Explicitly out of MVP: voice input UX and energy-aware scheduling behavior

### MVP.1

- LAN discovery, pairing, and selected-space sync
- Mutual confirmation code for pairing
- Encrypted transport
- Near-real-time sync with offline queue + replay
- Conflict policy: LWW by `updated_at` (UTC)
- Shared repeating occurrence semantics across paired devices

## Main quest model

- Main quest is derived by a pure getter over persisted data/events (not a separate mutable state machine)
- Manual `Set Main` is persisted via timestamp (`set_main_at`)
- Reminder-triggered focus is temporary preemption and can be stacked (LIFO)
- Reminder resolution unwinds to previous valid target
- Fallback priority (when no active override):
  1. Overdue quests
  2. Lower `order_rank`
  3. Priority
  4. `created_at` oldest first

Backlog ordering note:

- `order_rank` is in the domain/API model; drag-and-drop editing in the UI is deferred.

## Domain decisions

### IDs

- Quest ID is UUID primary key
- Space ID is string:
  - Reserved built-ins: `1` = Personal, `2` = Family, `3` = Work
  - Custom spaces: UUID

### Spaces

- Built-ins are non-deletable
- Built-in names are renamable
- Built-in rename propagates to paired devices
- Default quest space is `Personal` unless user picks another space

### Repeating quests

- Repeats use `series + occurrences`
- History lists each occurrence separately
- Shared completion is global per occurrence in synced spaces
- Duplicate occurrence prevention uses deterministic `occurrence_id` from `series_id + period_key`
- `period_key` uses UTC period boundaries

### Reminders

- Reminder firing can preempt Main quest
- If reminder target is already completed/abandoned, reminder is suppressed
- Snooze creates one-off reminder (10m, 30m, 1h), does not alter repeat cadence
- For date-only due values, local end-of-day is materialized to UTC
- Date NLP parsing is offline deterministic with preview + explicit confirm
- If notification permission is denied, show subtle visible warning and keep metadata editable

## Sync and collaboration (MVP.1)

- Local-first, accountless, cloudless
- Shared-space permissions: full read/write for paired devices
- Deletes are permanent and replicate globally (no resurrection)
- Completion actor metadata is stored (device hostname)
- Team completion updates are subtle in-app updates (not loud OS alerts)
- Completing shared occurrence cancels pending reminders for that occurrence on all peers
- Main-focus override data syncs across paired devices

## UI system direction

- Theme-ready architecture in MVP
- Theme format: token JSON mapped to CSS variables
- Include typography tokens in contract
- Motion values remain fixed (not tokenized yet)
- Theme preference is app-wide and device-local
- No accessibility guardrails are enforced at theme import level in this phase

## MCP policy

- MCP is MVP-critical
- Breaking MCP changes are allowed during MVP
- Every breaking change must ship with migration notes (old -> new mapping)

## Quality gates

### MVP gate

- Automated E2E: lifecycle + focus + reminders + MCP
- Manual smoke: Linux + Windows + Android

### MVP.1 gate

- Automated E2E: pairing + sync + shared repeat behavior

## Migration expectations

- Major schema changes must use real migrations
- API/storage should normalize legacy IDs into string format before lookup during transition

## Unresolved questions

- none
