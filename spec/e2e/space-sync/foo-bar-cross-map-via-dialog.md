---
title: Space Sync Foo Bar Cross-Map via Dialog
type: e2e
area: space-sync
author: user
---

# Space Sync Foo Bar Cross-Map via Dialog

## Goal

Validate that two custom spaces (`Foo`, `Bar`) can be mapped to different existing custom spaces on the paired device through incoming resolution, without forced same-name creation.

## Preconditions

- Device A and Device B are already paired.
- Both devices are online and Fini is running.
- Device A has custom spaces `Foo` and `Bar`.
- Device B has custom spaces `Foo` and `Bar` with independent local IDs.
- Device B can open `Settings -> Device/:id` for Device A.

## Test Steps

1. On Device A, open `Settings -> Device/:id` for Device B.
2. In `Mapped spaces`, select `Foo` and `Bar`, then tap `Save mappings`.
3. On Device B, open `Settings -> Device/:id` for Device A.
4. Wait for incoming custom-space resolution entries for remote `Foo` and remote `Bar`.
5. For remote `Foo`, choose `Select space to map`, then select local `Bar`.
6. For remote `Bar`, choose `Select space to map`, then select local `Foo`.
7. Confirm and apply both resolutions.
8. Reload mappings/status on Device B.
9. Reload mappings/status on Device A.
10. Reopen device detail on both sides and verify mapping persistence.

## Assertions

- Both incoming custom spaces are resolved through `Select space to map`.
- Remote `Foo` maps to local `Bar`.
- Remote `Bar` maps to local `Foo`.
- No duplicate custom space is created as a side effect.
- Existing `Foo` and `Bar` spaces remain present on both devices.
- No custom space is deleted or implicitly renamed.
- Mapping remains stable after reload/reopen on both devices.

## Failure Signals

- Incoming resolution UI does not appear for one or both remote spaces.
- `Select space to map` is unavailable and only create flow is possible.
- Cross-map selection is accepted but persisted mapping differs after reload.
- Additional unexpected custom spaces appear.
- Any existing `Foo` or `Bar` disappears after applying mappings.

## Evidence Artifacts

- DOM evidence for incoming resolution entries of remote `Foo` and remote `Bar`.
- DOM evidence for chosen cross-map targets (`Foo -> Bar`, `Bar -> Foo`).
- Post-apply DOM evidence on both devices showing persisted mappings.
- Structured command outputs/logs for mapping update and resolution application.
- Screenshot evidence only as rare fallback when DOM/state evidence is unavailable for a required assertion.

## Cleanup

1. Remove cross-mappings for `Foo` and `Bar` on both devices.
2. Restore mapping state to pre-test baseline.
3. Remove test-created spaces if they were created only for this test.
4. Reload mappings/spaces on both devices and verify baseline restoration.
5. Record cleanup evidence; if cleanup fails, mark the test as failed.
