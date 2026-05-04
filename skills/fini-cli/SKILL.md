---
name: fini-cli
description: "Shared foundation for using the Fini app binary and CLI interface. Use from other Fini skills when they need binary preflight, CLI/app mode selection, JSON vs human output decisions, runtime CLI smoke checks, or safe command sequencing. This is mostly a shared workflow layer; user-facing quest, space, reminder, and focus management should usually route through `fini`."
---

# Fini CLI Foundation

Use this skill when another Fini skill needs to operate or reason about the `fini` app binary and CLI interface.

This is a shared foundation skill. For user-facing quest, space, reminder, or focus management, route through `fini` first and use this skill for the CLI mechanics.

## Outcome

Use the Fini binary safely and predictably:

- Verify the binary is available before any action.
- Choose the correct app mode or CLI mode.
- Use deterministic JSON when a follow-up command depends on parsed output.
- Prefer human-readable output when reporting directly to the user.
- Stop early with concrete remediation when the binary or command surface is unavailable.

## Mandatory Preflight

Before any `fini` read or write action, run:

1. `command -v fini`
2. `fini --help`

If either check fails, stop and report:

- which check failed
- the exact command that failed
- the likely remediation, such as building or installing the app binary

Do not run quest, space, reminder, focus, or app commands until preflight succeeds.

## Binary Modes

`fini` is the single app binary. Its behavior depends on invocation:

| Invocation | Use |
|---|---|
| `fini` | Return the current Focus quest using CLI default behavior |
| `fini focus get` | Explicitly return the current Focus quest |
| `fini app` | Launch the GUI app |
| `fini quest ...` | List, get, create, update, complete, abandon, delete, or inspect quest history |
| `fini space ...` | List, create, update, or delete spaces |
| `fini reminder ...` | List, create, or delete reminders |

Use `fini app` when the user asks to open or launch the graphical app. Use CLI commands when the user asks for data, state changes, or automation.

## Output Mode

Use human-readable output by default when the command result is the final answer.

Use `--json` when:

- a later command depends on IDs or fields from the output
- comparing current state to requested state
- selecting a `space_id`, `quest_id`, or reminder target
- producing evidence for an automated workflow

Do not parse human-readable output when a JSON mode exists.

## Safe Command Sequencing

Before commands that need a space ID, run:

```text
fini space list --json
```

Resolve IDs from fresh CLI state rather than assuming built-in names still exist. Built-in spaces default to:

| ID | Default name |
|---|---|
| `1` | Personal |
| `2` | Family |
| `3` | Work |

Users may rename built-ins or create custom spaces, so prefer the fresh `space list` result over hard-coded names.

Before commands that update, complete, abandon, or delete an existing quest, get enough current state to confirm the target is unambiguous. Use `--json` when matching by title, status, space, due date, or other fields.

For destructive actions, confirm the target from current CLI state and use the most specific ID-based command available.

## Development And Runtime Checks

Use Makefile targets when validating the binary from this repo:

| Need | Command |
|---|---|
| Build release app binary | `make build` |
| Validate runtime container exposes CLI | `make runtime-smoke` |
| Build/update runtime container | `make runtime-image` |

Do not invent generic targets such as `make test`, `make lint`, or `make check` unless they exist in the current `Makefile`.

## Failure Handling

Prefer fail-fast behavior for:

- missing binary
- unavailable command surface
- invalid IDs
- ambiguous natural-language targets
- invalid state transitions
- missing required arguments

When a command fails, report:

- the failed command
- the relevant error text
- what state was already changed, if anything
- the next safest command to run

Do not continue with follow-up writes after a failed preflight or failed state-changing command unless the user explicitly asks for recovery.

## Reporting Pattern

When reporting CLI work, include:

- preflight status
- commands run, summarized without leaking sensitive local details
- final state or output
- any skipped actions or remaining ambiguity
