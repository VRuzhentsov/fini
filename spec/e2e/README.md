# E2E QA Guide

This folder contains ideal end-to-end QA specs for Fini features.

## Scope

- Validate feature behavior across real app surfaces.
- In most cases, run tests on two devices: one source device and one peer device.
- Treat the pair as the system under test, not a single isolated client.

## Feature Index

- `spec/e2e/device-connection/pairing-happy-path.md`
- `spec/e2e/space-sync/foo-create-via-dialog.md`
- `spec/e2e/space-sync/foo-bar-cross-map-via-dialog.md`
- `spec/e2e/skill/README.md`
- `spec/e2e/cli/README.md`

## Default Two-Device Topology

- Device A: action source (creates/updates state).
- Device B: receiving peer (verifies incoming behavior).
- For sync cases, verify both directions by reloading and reopening relevant screens on both devices.

## Evidence Policy (DOM-First)

- Primary evidence: DOM snapshots and structured state outputs.
- Supporting evidence: command outputs and logs tied to the specific action.
- Screenshot evidence is a rare edge-case fallback only when required DOM/state evidence is unavailable.

## QA Flow Used in Most Cases

1. Establish baseline state on Device A and Device B.
2. Capture baseline DOM/state evidence on both devices.
3. Execute action on Device A.
4. Verify expected state change on Device B.
5. Reload/reopen relevant screens on both devices and verify persistence.
6. Verify safety conditions (no silent deletion, no unintended remap, no hidden side effects).
7. Run mandatory cleanup and verify baseline restoration on all participating devices.

## Mandatory Cleanup (Required in Every E2E Test Case)

- Remove all test-created data (spaces, quests, reminders, mappings) from all participating devices.
- Restore pairing and mapping state to the original pre-test baseline.
- Exit transient modes used only for the test.
- Re-run baseline checks to prove cleanup completion.
- If cleanup is incomplete, mark the test as failed.

## Android Testing Details

- Use Android as one side of the two-device setup for mobile behavior validation.
- Connect and launch with:
  - `make android-connect`
  - `make android-launch`
- Keep the app foregrounded during scenario execution.
- Collect DOM/state evidence for each assertion; use screenshots only for rare fallback cases.

## Linux Testing Details

- Use Linux desktop as the second side for cross-device checks.
- Run desktop app with `make dev`.
- Collect DOM/state evidence for each assertion; use screenshots only for rare fallback cases.

## Reporting Expectations

- Report action-by-action outcomes with linked evidence.
- Separate proven outcomes, blocked outcomes, and unproven assumptions.
- Highlight data-loss risks immediately (for example: missing space, unexpected merge, hidden deletion).
