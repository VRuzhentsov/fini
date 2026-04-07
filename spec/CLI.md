# CLI

## Purpose

Define the Fini CLI as the primary synchronous interface for automation and skill-driven actions.

## Scope

- CLI command contract and output policy.
- Full operation coverage across quest, space, reminder, focus, device, and sync domains.
- Skill preflight requirements for binary accessibility.

## Non-goals

- Removing or changing MCP behavior in this spec.
- Replacing GUI behavior outside explicit CLI entry contracts.

## Entry Contracts

`fini` is globally accessible on Linux through PATH and supports these top-level invocations:

| Invocation | Behavior |
|---|---|
| `fini` | Return current Focus quest (human output by default) |
| `fini --help` | Show full command list and usage |
| `fini app` | Launch GUI app |
| `fini mcp` | Start MCP server over stdio (compatibility path retained) |

## Global Binary Accessibility (Linux)

- `command -v fini` must resolve to an executable binary.
- `fini --help` must execute successfully.
- Packaging/installation must keep `fini` available across new shell sessions.

## Output and Exit Code Policy

- Default output is human-readable.
- `--json` is supported for machine consumers on all commands.
- Errors print concise human text to stderr and structured payload with `--json`.
- Exit codes are stable:
  - `0` success
  - `2` invalid args/usage
  - `3` not found
  - `4` invalid state transition
  - `5` runtime/system failure

## Command Surface (Full Parity Target)

### Focus

- `fini` (alias of `fini focus get`)
- `fini focus get [--json]`
- `fini focus set --quest-id <id> [--trigger <manual|reminder>] [--json]`

### Quest

- `fini quest list [--status <active|completed|abandoned>] [--space-id <id>] [--json]`
- `fini quest get --id <id> [--json]`
- `fini quest create ... [--json]`
- `fini quest update --id <id> ... [--json]`
- `fini quest complete --id <id> [--json]`
- `fini quest abandon --id <id> [--json]`
- `fini quest delete --id <id> [--json]`
- `fini quest history [--json]`

### Space

- `fini space list [--json]`
- `fini space create --name <name> [--json]`
- `fini space update --id <id> [--name <name>] [--order <n>] [--json]`
- `fini space delete --id <id> [--json]`

### Reminder

- `fini reminder list --quest-id <id> [--json]`
- `fini reminder create --quest-id <id> --type <relative|absolute> ... [--json]`
- `fini reminder delete --id <id> [--json]`

### Device Connection

- `fini device identity [--json]`
- `fini device add-mode enter|leave [--json]`
- `fini device discovery [--json]`
- `fini device presence [--json]`
- `fini device pair send --request-id <id> --to-device-id <id> --to-addr <ip> [--json]`
- `fini device pair incoming|outgoing-updates|outgoing-completions [--json]`
- `fini device pair accept --request-id <id> [--json]`
- `fini device pair complete --request-id <id> [--json]`
- `fini device pair acknowledge --request-id <id> [--json]`
- `fini device paired list|save|unpair ... [--json]`
- `fini device updates consume-space-mapping [--json]`
- `fini device debug status [--json]`

### Space Sync

- `fini sync mappings list --peer-device-id <id> [--json]`
- `fini sync mappings update --peer-device-id <id> --mapped-space-id <id>... [--json]`
- `fini sync mappings apply-remote --peer-device-id <id> --mapped-space-id <id>... [--json]`
- `fini sync mappings resolve-custom --peer-device-id <id> --space-id <id> --mode <create_new|use_existing> [--existing-local-space-id <id>] [--json]`
- `fini sync tick [--peer-device-id <id>] [--json]`
- `fini sync status [--peer-device-id <id>] [--json]`

## MCP-to-CLI Parity Matrix

MCP behavior remains implemented. CLI must provide an equivalent command for each existing MCP operation.

| Current MCP Tool | Target CLI |
|---|---|
| `list_quests` | `fini quest list` |
| `get_quest` | `fini quest get --id` |
| `get_active_focus` | `fini` / `fini focus get` |
| `create_quest` | `fini quest create` |
| `update_quest` | `fini quest update --id` |
| `complete_quest` | `fini quest complete --id` |
| `abandon_quest` | `fini quest abandon --id` |
| `delete_quest` | `fini quest delete --id` |
| `list_history` | `fini quest history` |
| `list_spaces` | `fini space list` |
| `create_space` | `fini space create --name` |
| `update_space` | `fini space update --id` |
| `delete_space` | `fini space delete --id` |
| `list_reminders` | `fini reminder list --quest-id` |
| `create_reminder` | `fini reminder create --quest-id` |
| `delete_reminder` | `fini reminder delete --id` |

## Unified Action Service

Define a single shared service/module as the execution core for all interfaces.

- CLI and MCP both call the same action service methods.
- GUI/Tauri commands call the same action service methods.
- Business rules and persistence logic are implemented once in that shared module.
- Interface adapters (CLI/MCP/GUI) only handle argument parsing, transport, and output formatting.
- No interface-specific divergence in mutation or validation logic.

## Skill Integration Contract

Any skill that controls Fini must pass this preflight before performing actions:

1. `command -v fini`
2. `fini --help`

If either check fails, stop immediately, report failure, and provide concrete PATH/install remediation.

## Acceptance Criteria

- `fini` is globally executable on Linux.
- `fini --help` lists command groups and examples.
- `fini` returns current Focus quest.
- `fini app` launches GUI.
- CLI command surface reaches full parity with listed MCP operations.
- Skill preflight is documented and enforced before any action.
