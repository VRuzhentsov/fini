# Interface Parity Feature E2E

This feature defines end-to-end QA for interface-agnostic behavior in Fini.

## Goal

Validate that CLI and MCP perform the same actions, produce equivalent state changes, and stay aligned through one shared action service.

## Core Principle

- Interface is transport only.
- Action logic is shared.

CLI and MCP must call the same service/module for quest, space, reminder, focus, device, and sync operations.

## Core Scenarios

1. Run the same quest lifecycle via CLI and MCP, compare persisted results.
2. Run the same space CRUD operations via CLI and MCP, compare results and ordering.
3. Run reminder operations via CLI and MCP, compare resulting reminders.
4. Run focus operations via CLI and MCP, compare active Focus result.
5. Run representative device/sync flows via CLI and MCP, compare mapping and status outputs.

## Assertions

- Same inputs produce equivalent outputs across CLI and MCP.
- Persisted database state is equivalent regardless of interface.
- Validation errors are equivalent in meaning and constraints.
- No interface introduces side effects absent in the other.

## Evidence

- CLI command transcripts and outputs.
- MCP request/response payloads.
- DOM/state evidence for user-visible claims.
- Screenshot evidence only as rare fallback when DOM/state evidence is unavailable.

## Cleanup

- Remove all test-created data from all participating devices.
- Restore baseline pairing and mapping state.
- Verify baseline restoration before closing the test case.
