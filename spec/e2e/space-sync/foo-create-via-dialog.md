---
title: Space Sync Foo Create via Dialog
type: e2e
area: space-sync
author: user
---

# Space Sync Foo Create via Dialog

## Goal

Validate that a custom space created on one device (`Foo`) is offered to the paired device through the incoming space resolution dialog, and the paired device can create it locally via `Create`.

## Preconditions

- Device A and Device B are already paired.
- Both devices are online and Fini is running.
- Neither device currently has a custom space named `Foo`.
- Device B can open `Settings -> Device/:id` for Device A.

## Test Steps

1. On Device A, open `Settings -> Spaces`.
2. Create a new custom space named `Foo`.
3. On Device A, open `Settings -> Device/:id` for Device B.
4. In `Mapped spaces`, select `Foo` and tap `Save mappings`.
5. On Device B, open `Settings -> Device/:id` for Device A.
6. Wait for incoming custom-space resolution prompt for `Foo`.
7. In the dialog, keep mode `Create` and confirm.
8. On Device B, reload mappings/status.
9. On Device B, open `Settings -> Spaces` and confirm `Foo` exists.
10. On Device A, reload `Settings -> Spaces` and confirm `Foo` still exists.

## Assertions

- Device B shows incoming custom-space resolution UI for `Foo`.
- Choosing `Create` results in a local space on Device B with name `Foo`.
- `Foo` becomes available in mapped spaces for the device pair.
- `Foo` remains present after reload/reopen on both devices.
- No existing space is removed as a side effect of this flow.

## Failure Signals

- No incoming dialog appears on Device B.
- `Foo` mapping is applied but `Foo` is absent in Device B spaces list.
- `Foo` appears briefly and disappears after refresh.
- Selecting `Create` unexpectedly merges into or deletes another local space.

## Evidence Artifacts

- Device A DOM evidence: `Foo` creation and `Save mappings` enabled/success state.
- Device B DOM evidence: incoming `Foo` dialog, `Create` confirmation, spaces list containing `Foo`.
- Post-reload DOM evidence from both devices showing `Foo` remains present.
- Structured command outputs/logs around mapping update, incoming update consumption, and resolution command.
- Screenshot evidence only as rare fallback when DOM data is unavailable for a required assertion.

## Cleanup

1. Remove `Foo` from mapped spaces on both devices.
2. Delete test-created `Foo` spaces from both devices.
3. Reload spaces and device detail on both sides to verify baseline restoration.
4. Record cleanup evidence; if cleanup fails, mark the test as failed.
