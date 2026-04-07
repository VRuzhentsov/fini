---
title: Space Sync Quest Between Spaces
type: e2e
area: space-sync
author: opencode
---

# Space Sync Quest Between Spaces

## Goal

Validate quest synchronization behavior when a single quest moves between mapped and unmapped spaces, then returns to mapped scope.

## Preconditions

- Device A and Device B are already paired.
- Both devices are online and Fini is running.
- Pair mapping baseline is `Personal (1)` and `Family (2)` enabled; `Work (3)` disabled.
- No existing test quest with the planned unique title on either device.

## Test Steps

1. On Device A, open `Personal` and create a quest with unique title (for example `SYNC-MOVE-<timestamp>`).
2. Trigger sync (`space_sync_tick`) and confirm the quest appears on Device B in `Personal`.
3. On Device A, move the same quest from `Personal` to `Family`.
4. Trigger sync and confirm Device B shows the same quest `id` in `Family`.
5. On Device A, move the same quest from `Family` to `Work` (currently unmapped).
6. Trigger sync and confirm Device B no longer shows that quest in mapped lists (`Personal`/`Family`).
7. On Device A, enable `Work` mapping and save mappings.
8. Trigger sync and confirm Device B now shows the quest in `Work`.
9. Reload/reopen relevant screens on both devices and confirm state persists.

## Assertions

- Quest create in mapped space replicates to peer.
- Quest move mapped -> mapped converges with unchanged quest `id` and updated `space_id`.
- Quest move mapped -> unmapped does not leave stale peer copy in mapped spaces.
- Enabling mapping for previously unmapped destination space backfills quest visibility.
- State remains consistent after reload/reopen on both devices.

## Failure Signals

- Quest created on Device A never appears on Device B for mapped space.
- Quest appears duplicated instead of moving when `space_id` changes.
- Quest remains visible on Device B after mapped -> unmapped move.
- Enabling `Work` mapping does not surface the moved quest on Device B.
- Reload/reopen changes the result unexpectedly.

## Evidence Artifacts

- Device A and Device B DOM/state evidence before first action.
- Structured state outputs after each mutation (`get_quests`, `space_sync_status`, mapping state).
- Evidence chain per step: write action -> persisted origin state -> peer read state.
- Screenshot evidence only as fallback when MCP DOM/state evidence is unavailable.

## Cleanup

1. Delete the test quest from all devices where it exists.
2. Restore mapping baseline to `1,2` only (disable `Work` if enabled for the test).
3. Reload/reopen on both devices and verify no leftover test quest.
4. Record cleanup evidence; if cleanup fails, mark the test as failed.
