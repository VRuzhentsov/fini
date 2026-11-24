# Fini bridge for Claude Code

Native Claude Code plugin (tested with **2.1.330**). Node **24.12+**, POSIX and
util-linux `flock` are required, exactly as for the shared core. No npm production
dependencies, MCP server, service, shell-interpolated arguments or configurable
executable. This implementation does not install or enable itself.

## Build and use

From `src-agent-plugin/`:

```bash
npm run package:claude
claude plugin validate --strict .artifacts/claude/fini
claude --plugin-dir "$PWD/.artifacts/claude/fini"
```

In that Claude session, type:

```text
/fini:link 001
/fini:sessions 001
```

The **entire native command_args string** is the quest ID: it is neither trimmed,
split nor interpreted as JSON. It must be explicit and nonblank. Leading zeros,
spaces, case and punctuation are preserved as supplied by Claude. Claude itself
may parse whitespace before delivering command_args; we cannot recover characters
it has discarded. Quotes are literal, not a quoting syntax. There is no active
quest inference, Fini lookup, quest creation, progress update or Jira write.

`link` binds the current runtime-supplied Claude UUID, never a UUID argument.
`sessions` lists **all** Hermes and Claude associations for the selected quest;
`sessions` entries each have `provider` and `session_id`. `session_ids` is the
legacy Hermes-only field. Results are JSON displayed in a native hook block
message (even on success); `added: false` means an idempotent repeat. A block is
intentional: no prompt expansion or paid model turn is needed for the operation.

## One shared store; explicit profile boundary

Default Claude storage is the existing Hermes convention:

```text
${HERMES_HOME:-$HOME/.hermes}/plugin-data/fini-hermes-plugin/sessions.json
```

This is **not a Claude-specific data file**. With no overrides, it addresses the
same default Hermes association file, without moving/copying existing data. There
is no fixed username or absolute home path in code. Both bridges support the
same optional environment variable:

```bash
export FINI_SESSIONS_PATH="/absolute/approved/hermes-home/plugin-data/fini-hermes-plugin/sessions.json"
```

Set it in the environment of **both newly launched runtimes** to share a selected
store. It must name an absolute local file, not a directory or command. Blank and
relative values fail before writes, with no fallback. Hermes' unset default still
comes from `get_hermes_home()` on every call, preserving its context-local/profile
selection. Claude cannot discover another running Hermes process's active profile.
For custom profiles/context-local homes, explicitly pass the exact approved path;
do not guess from the current directory or search/merge profiles. Claude rejects
non-default `HERMES_PROFILE` or `HERMES_DATA_DIR_SUFFIX` unless the explicit shared
path is set. `HERMES_HOME`, if used by Claude, must be absolute (no tilde expansion).
A shell without these overrides always means default, not whichever profile was
last used elsewhere. Review `state_path` in the result before relying on a link.

This repository change makes no environment/config changes and never relocates
or edits live state during validation. The existing Hermes installation stays as
it is. Explicitly choosing a common path is the only cross-profile
opt-in. The shared core provides schema validation, literal IDs, mixed-provider
dedup, migration, lock timeouts and atomic persistence; see the root README.

## Cached installation

**Do not install this source subdirectory directly.** The sole source of domain
logic is `../shared/*.mts`, outside this source adapter directory. Claude caches
only its installed plugin root. `npm run package:claude` assembles a self-contained
root at `.artifacts/claude/fini` containing the exact shared files and adapter,
plus the native manifest, hooks and skills at their standard paths. Relative
imports remain inside that assembled root. No symlinks, source-checkout fallback,
absolute code path or on-install network/build step is needed in the cache.
`source-hashes.json` records canonical source hashes; the relocation test compares
every packaged module byte-for-byte. Generated artifacts are ignored, never
hand-edited or maintained as another implementation. Rebuild after source changes;
installed versions are snapshots, not automatically live source updates.

An optional **local** marketplace is generated beside it. Installation modifies
Claude settings, so run these only when you choose to activate it:

```bash
claude plugin marketplace add "$PWD/.artifacts/claude"
claude plugin install fini@fini-local --scope user
```

For copied/versioned source updates, bump the manifest version, rebuild (the
marketplace version is derived automatically), then use Claude's documented
marketplace/plugin update flow and restart. Relative-path local marketplaces may
load the assembled root in place; rebuild and reload/restart in that case.
No remote distribution or plugin auto-update is configured here. Session-only
`--plugin-dir` is sufficient and avoids persistent installation.

## Trusted context and failure boundaries

The native `UserPromptExpansion` event fires on **user-typed** commands. Its
`session_id` is documented as the current runtime session; `command_name`,
`command_source`, `expansion_type` and `command_args` are also native fields. The
bridge requires that event, plugin source, slash-command expansion and either
`fini:link` or `fini:sessions`. It forwards only `session_id` from the hook to the
shared core, which requires an exact UUID before writing. Missing/invalid identity
fails closed; no environment session-ID, transcript scan, model-filled argument
or SessionStart state cache exists. Skills disable model invocation. Fork/resume
use the ID supplied on that invocation, not a previous session's ID.

The hook always returns `decision: block`, with the deterministic core JSON as
`reason`, so both results and errors go to the user. Skills contain only an
unavailable-hook stop instruction as a fallback. Disabled hooks, `--bare`, older
Claude versions without UserPromptExpansion, another hook blocking first or a
launcher failure cannot create a link: there is no executable body in the skill.
Claude may continue expansion/model processing on a hook process failure; this is
not a sandbox or a guarantee of a model-free turn when hooks malfunction. Do not
use unsupported clients for linking. Tested native version is 2.1.330; no minimum
version was guessed. This is Claude Code, not claude.ai/Cowork support.

A same-user local process can forge hook JSON or directly call the shared CLI.
The bridge relies on native invocation, not cryptographic attestation, and is not
an adversarial same-user authentication boundary. It ignores all transcript,
prompt and extra session/quest hints, and does not infer identity from them.

Input is limited to 1 MiB and a 15-second EOF deadline. The core has an 8-second
lock wait / 12-second worker deadline; native hook timeout is 20 seconds. Result
size is bounded. Errors after an atomic commit can leave a successful write:
inspect state before retrying. Provider/session dedup makes retries idempotent.

## Verification and official contracts

```bash
npm run check                 # strict TS, core, Claude bridge and Hermes suites
npm run test:claude:native     # optional: real Claude CLI, Python3 ZIP creation
npm run package:claude
claude plugin validate --strict .artifacts/claude/fini
```

Automated tests run real bridge processes against disposable common storage,
including relocated cache layout and mixed Hermes/Claude writers. Native manifest
validation is separate from interactive acceptance. No paid/live user conversation
is used by these tests. The optional native probe loads a real ZIP via Claude's
supported `--plugin-dir` path under a temporary HOME/CLAUDE_CONFIG_DIR. It uses no
user credentials and points the API at an unreachable loopback endpoint. Real
CLI-generated session IDs must match exact readback; resume dedup and mixed
session listing must succeed with zero turns, zero API duration and zero cost.
This passed on Claude 2.1.330. Live interactive frontend acceptance remains
unperformed; neither plugin was activated in the user's configuration.

`claude plugin validate --strict` passes the runtime manifest/component checks.
It also prints separate catalog-submission diagnostics: no listing icon is
included. The validator reads the repository-root license when one is available.
These checks do not block local loading, but this plugin
is **not prepared for public directory submission**. A license choice requires
the owner's decision; this implementation does not invent one.

Official references (consulted during implementation):

- https://code.claude.com/docs/en/hooks#userpromptexpansion
- https://code.claude.com/docs/en/hooks#common-input-fields
- https://code.claude.com/docs/en/skills#control-who-invokes-a-skill
- https://code.claude.com/docs/en/plugins-reference
- https://code.claude.com/docs/en/plugins/create
- https://code.claude.com/docs/en/plugins/loading

Packaging lesson: keep shared imports inside the *assembled installed root*, not
just valid relative to the development checkout; always test a relocated copy.
Identity lesson: a format-valid UUID is not identity proof; only the native
hook context selects the current session. Never move that choice into a prompt.
