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
- `spec/e2e/space-sync/quest-sync-between-spaces.md`
- `spec/e2e/skill/README.md`
- `spec/e2e/cli/README.md`
- `spec/e2e/interface/README.md`

## Default Two-Device Topology

- Device A: action source (creates/updates state).
- Device B: receiving peer (verifies incoming behavior).
- For sync cases, verify both directions by reloading and reopening relevant screens on both devices.

## Execution Policy (MCP-First)

- Prefer Tauri MCP for all app interaction and verification on both devices.
- Required pre-check before each scenario:
  1. `driver_session status`
  2. `ipc_get_backend_state`
- Treat backend identity as source of truth for target classification:
  - Android target: `environment.os = android`, `environment.arch = aarch64`
  - Linux target: `environment.os = linux`, `environment.arch = x86_64`
- Use MCP interaction/state tools first (`webview_interact`, `webview_wait_for`, `webview_dom_snapshot`, `webview_execute_js`, `ipc_*`).
- Use raw Android `adb input tap` only as a last-resort fallback when MCP cannot reliably operate a control.

## Evidence Policy (State-First)

- Primary evidence: DOM snapshots and structured state outputs.
- Supporting evidence: command outputs and logs tied to the specific action.
- Screenshot evidence is a last-resort fallback only when required DOM/state evidence is unavailable.
- For each assertion, capture an evidence chain: write action -> persisted state -> read verification.
- If fallback evidence is used (screenshot/tap/log-only), record why MCP/state-first evidence was insufficient.

## QA Flow Used in Most Cases

1. Establish baseline state on Device A and Device B.
2. Capture baseline DOM/state evidence on both devices.
3. Execute action on Device A.
4. Verify expected state change on Device B.
5. Verify persistence by reloading/reopening relevant screens on both devices.
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
- Drive Android primarily via Tauri MCP, not raw ADB input.
- Use Android screenshots only as fallback evidence when MCP DOM/state evidence is unavailable or incomplete.
- Use Android tap coordinates only as fallback interaction when MCP cannot activate the intended control.
- When fallback is used, include reason + before/after artifacts in the report.

## Linux Testing Details

- Use Linux desktop as the second side for cross-device checks.
- Run desktop app with `make dev`.
- Collect DOM/state evidence for each assertion; use screenshots only for rare fallback cases.

## Reporting Expectations

- Report action-by-action outcomes with linked evidence.
- Separate proven outcomes, blocked outcomes, and unproven assumptions.
- Highlight data-loss risks immediately (for example: missing space, unexpected merge, hidden deletion).
