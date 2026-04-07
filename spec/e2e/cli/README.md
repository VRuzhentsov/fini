# CLI Feature E2E

This feature defines ideal end-to-end QA coverage for CLI-first operation in Fini.

## Goal

Validate that CLI commands are the primary synchronous control surface for product operations and produce deterministic, testable outcomes.

## Entry Behavior

- `fini` returns current Focus quest.
- `fini --help` shows command groups and usage.
- `fini app` launches GUI explicitly.

## Ideal CLI Contract

- Commands are explicit, synchronous, and idempotent where applicable.
- Output is structured and stable for automation.
- Exit codes cleanly separate success from failure.
- Error responses include machine-readable codes and human-readable context.

## Target Command Areas

- Quest lifecycle (`list`, `get`, `create`, `update`, `complete`, `abandon`, `delete`).
- Space management (`list`, `create`, `update`, `delete`).
- Reminder management (`list`, `create`, `delete`).
- Focus and history queries.
- Sync and mapping operations for paired devices.

## Core Scenarios

1. Happy-path create/update/read/delete flows for each command area.
2. Validation failures (missing args, invalid ids, invalid state transitions).
3. Repeat executions to confirm idempotency behavior.
4. Cross-device commands where output must reflect synchronized state.

## Assertions

- Command output matches requested operation.
- Exit code policy is consistent.
- Data state is correct after reload/reopen checks.
- No unrelated records change.

## Binary Accessibility Preflight

Run before any CLI test case:

1. `command -v fini`
2. `fini --help`

If either check fails, stop the test and mark it blocked by environment setup.

## Evidence

- Exact CLI command transcript.
- Structured command output and exit codes.
- DOM/state evidence only for claims about visible UI state.
- Screenshot evidence only as rare fallback when DOM/state evidence is unavailable.

## Cleanup

- Remove all test-created records from all participating devices.
- Restore baseline pairing and mapping state.
- Verify baseline restoration before closing the test case.
