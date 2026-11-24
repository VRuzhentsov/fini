# Fini agent plugins

Link an explicitly selected Fini quest ID to an agent session, then list its
linked sessions across Hermes and Claude Code. Linking is not quest completion.
The current plugins store associations locally. They do not call Fini or check
whether a quest exists.

Run the commands on this page from `src-agent-plugin/`, not from the Fini
repository root. Agent plugins have no CI checks at present. Runtime-specific
details are in the [Hermes guide](fini-hermes-plugin/README.md) and
[Claude guide](fini-claude-plugin/README.md).

## Source tree

```text
shared/
  core.mts           # validation, command decoding, schema, domain and lock runner
  store.mts          # internal locked transaction, migration and atomic persistence
  cli.mts            # public one-request JSON stdin/stdout executable
  locked-worker.mts  # internal flock-owned child (not a public entrypoint)
  transport.mts      # bounded JSON transport
  tests/core.test.mts
fini-hermes-plugin/   # thin Python registration/context/subprocess bridge
fini-claude-plugin/   # native Claude hook/skills + reproducible cache packaging
```

- **Shared core:** the sole owner of validation, schema, migrations, deduplication,
  locking and persistence. All bridges use one provider-qualified quest-session
  store, supporting multiple sessions per quest across agents.
- **Native bridges:** own runtime session identity and native commands/UX, then
  delegate to the core through a thin local CLI JSON boundary. No server is
  required. Installation artifacts package the canonical shared core alongside
  each bridge rather than maintaining divergent copies.
- **Quest association:** users explicitly select the quest. Session membership
  is navigation evidence, not inferred quest progress or completion.
- **Shared storage:** target an explicitly selected, agent-neutral path outside
  the code repository. The current Hermes-default store remains a compatibility
  default for both bridges; an explicit shared path is already supported. The
  target storage design below is not implemented. No live store is moved.

This source import keeps the nested npm package and its lockfile separate from
the frontend package. It changes no installed plugin, config or live state.
The standalone checkout and its Git history stay in place for review.

## Current commands

| Runtime | Interface | Result |
|---|---|---|
| Hermes | `/fini-link <quest-id>` | Link the calling Hermes session |
| Hermes | `/fini-sessions <quest-id>` | List Hermes and Claude links for the quest |
| Hermes | `/fini-link-claude {"quest_id":"<quest-id>","session_id":"<claude-uuid>"}` | Link an explicitly supplied Claude UUID |
| Hermes toolset `fini` | `fini_link_current_session` with `{"quest_id":"<quest-id>"}` | Link the calling agent's runtime session; a child links itself, not its parent |
| Claude Code | `/fini:link <quest-id>` | Link the current native Claude session |
| Claude Code | `/fini:sessions <quest-id>` | List Hermes and Claude links for the quest |

Current-session commands take identity from the runtime, not from model arguments.
The explicit Hermes command for a Claude UUID checks its format, not its existence.
See the [Hermes contract](fini-hermes-plugin/README.md#commands) and
[Claude contract](fini-claude-plugin/README.md#build-and-use) for exact argument rules.

## Target design — approved, not implemented

The approved direction is one shared association store with SQLite and Drizzle,
separate native Hermes and Claude plugins, and native `/fini:*` commands for
link, unlink, primary-session selection and relations. These are target features,
not the current command contract. Current storage is locked JSON; there is no
Drizzle dependency, SQLite schema, unlink command or primary-session behavior here.
The existing Claude `/fini:link` and `/fini:sessions` commands do not implement
the full target interface.

In the target design, Fini access is through the Fini CLI only. The association
store holds quest-session links; it is not direct access to the Fini app database.
The current shared JSON executable described below is a plugin entry point,
not the Fini app CLI. Current plugins do not read or change Fini records.

Reminder import is separate work and is not included in this source tree.

## Runtime and installation

Requires **Node.js 24.12+**, **util-linux `flock` on PATH**, POSIX process groups
and a local filesystem supporting `flock`, atomic rename and directory `fsync`.
Node runs `.mts` as ESM through native type stripping. No compiler, TypeScript
loader, server or service is required at runtime. Bun is not supported.
Keep the Hermes plugin and its shared sibling together when installing.

There are **no npm production dependencies**. `typescript`, `@types/node` and
their transitive development dependencies are locked:

```bash
npm ci
npm run check
```

The Python bridge requires the installed Hermes plugin API. Its fixed reviewed
command is `node <resolved-repository>/shared/cli.mts`, not a configurable shell
command. This source import does not change an installed plugin link.
To install this source, point the Hermes plugin link at
`<fini-checkout>/src-agent-plugin/fini-hermes-plugin`.
Keep its sibling `shared/` folder available. Changing that link or enabling
the plugin requires separate approval and a new Hermes process. See
[Hermes commands and storage](fini-hermes-plugin/README.md).

The native [Claude Code bridge](fini-claude-plugin/README.md) provides
`/fini:link <quest-id>` and `/fini:sessions <quest-id>` using the runtime's actual
hook session ID. Build the self-contained cacheable plugin with
`npm run package:claude`; load `.artifacts/claude/fini`, **not** the source
adapter subdirectory. No Claude settings or live associations are changed by
building or testing. `npm run test:claude:native` exercises real Claude ZIP
loading, current-runtime identity, resume dedup and listing in an isolated home
with a dead local API endpoint and no user credentials/model turns.

## Shared core contract

From this package, send one JSON document on stdin to `node shared/cli.mts`.
The executable emits one JSON object plus newline on stdout and exits; no daemon.
For example (use a disposable absolute path when experimenting):

```json
{"action":"link","state_path":"/absolute/approved/storage/sessions.json","quest_id":"001","provider":"hermes","session_id":"runtime-owned-id"}
```

```json
{"action":"link","state_path":"/absolute/approved/storage/sessions.json","quest_id":"001","provider":"claude","session_id":"60eba7ee-1dd7-48fb-969d-e68a263ed0a0"}
```

```json
{"action":"list","state_path":"/absolute/approved/storage/sessions.json","quest_id":"001"}
```

Only those fields are accepted. The module API `execute(unknown)` in `core.mts`
uses the identical validation/locking/response boundary. The Claude adapter
supplies its native hook's runtime ID and the selected shared storage path.
It does **not** unify profiles or move/migrate live storage now.
Never call `locked-worker.mts` or `transaction()` directly; those are internal
unlocked implementation surfaces, not adapter APIs.

The Hermes transport form is exactly `command`, `raw_args`, `session_id`,
`state_path`. Commands are `fini-link`, `fini-link-claude`, `fini-sessions`.
Only Hermes' actual invocation-context `session_id` goes into `fini-link`.
The assistant-callable `fini_link_current_session` tool accepts only `quest_id`
and forwards the per-call runtime keyword `session_id` to that same core command.
It links the caller (a delegated child's ID is not its parent's); missing runtime
identity fails closed. See the Hermes README for traced API call sites and the
separate plugin/toolset activation boundary. No live activation is performed.
The core, not Python, parses/validates raw slash arguments and external UUIDs.
Every adapter must own proof of its runtime identity; the local executable
validates input shape, not the existence or ownership of a supplied session.

Results retain `quest_id`, `session_ids` (Hermes-only), `sessions`
(provider/session objects), `state_path`, and for links `provider`, `session_id`,
`added`. Errors are `{"error":"..."}` and also exit 0 when handled; callers must
inspect `error`. Interpreter/launcher failures can exit nonzero. Error text for
malformed JSON or OS errors is not a stable API; result fields and command syntax
are unchanged. IDs are preserved literally, including whitespace and UUID case.

## Locking, limits and failure boundaries

The TypeScript core validates before creating directories. Python resolves
`get_hermes_home()/plugin-data/fini-hermes-plugin/sessions.json` on each call
unless `FINI_SESSIONS_PATH` explicitly selects an absolute shared file, matching
`plugin_data_dir()` without its eager mkdir side effect. Claude defaults to the
same default Hermes store (or absolute `HERMES_HOME`) and supports that same
explicit override; it does not discover or merge other runtimes' profiles.
All validation,
schema handling, migration, listing, deduplication, reads and writes are TypeScript.
No duplicate Python persistence logic remains.

The core launches fixed `flock --exclusive --timeout 8 --conflict-exit-code 75
--no-fork <directory>/sessions.lock <current-node> shared/locked-worker.mts`.
The worker inherits the kernel-owned lock until exit, including on SIGKILL.
The stable sidecar is **never removed or stale-broken**; this interoperates with
the previous Python `fcntl.flock()` writer. All participants must use this same
sidecar; local manual edits or bypassing the public API are not protected.
No network filesystems or Windows support is claimed.

Writes create a unique same-directory mode-0600 temporary file, flush/fsync it,
rename atomically, fsync the directory and clean the temporary file on handled
failure. Existing corrupt/unsupported state is never overwritten. V1 is read in
memory as Hermes-only; only the next successful link (even a duplicate) writes
v2. Listing and v2 duplicate links never rewrite the JSON. A killed process can
leave an unrenamed temporary file; it does not strand a kernel lock.

Stdin/stdout documents are limited to 1 MiB. Worker lock wait is 8 seconds; the
core bounds its worker to 12 seconds; Python bounds its subprocess tree to 15
seconds and aggregate stdout/stderr to 1 MiB. Direct stdin clients must send EOF
(and impose their own caller deadline). The state file is not size-capped, but
an excessively large listing returns an error. A late transport/timeout/fsync
failure can occur after an atomic commit: read the exact state before retrying;
provider/session dedup makes repeating a link idempotent. These are local
same-user executable/file boundaries, not authentication or a sandbox.

## Verification and references

```bash
npm run typecheck       # strict production AND test TypeScript
npm test                # direct CLI/core, real independent processes and OS lock
npm run test:hermes     # installed Hermes PluginContext/dispatch through Node
npm run test:claude     # native representative hook + relocated cache layout
npm run package:claude  # assemble the Claude plugin cache
npm run test:claude:native # optional real Claude CLI ZIP/session-ID acceptance
```

There is no Make target for these checks. `npm run check` runs TypeScript,
shared-code, Claude and Hermes checks together. Run `npm run package:claude`
separately for package assembly. TypeScript, shared-code and Claude hook tests
need no Hermes or Claude binary. Hermes tests need the installed Hermes runtime.
The native Claude check also needs the Claude CLI and Python 3 for ZIP assembly.
After package assembly, validate the manifest locally with:

```bash
claude plugin validate --strict .artifacts/claude/fini
```

Tests use isolated `HERMES_HOME` directories under `$TMPDIR` (set it to a scratch
location before running outside this workstation). They preserve literal IDs,
multiple sessions/providers, migration, profile isolation, corrupt bytes,
replacement failure, lock contention/owner death, and installed symlink routing.
The Hermes suite requires an installed Hermes runtime on PATH. It registers
this copied plugin through the real PluginContext in temporary homes, then
calls the shared TypeScript code. Its temporary plugin link points at this
source, not at an operator's installed plugin. It changes no live installation.
The native Claude check uses an isolated home and an unreachable API endpoint.
It checks runtime IDs, resume, duplicate links and listing without model turns.
Report local results separately from interactive acceptance. These checks do
not install or enable either plugin.

Official sources consulted for this rewrite:

- https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/
- https://nodejs.org/api/typescript.html
- https://raw.githubusercontent.com/util-linux/util-linux/master/sys-utils/flock.1.adoc

Installed Hermes source was also traced: `cli.py` passes `self.session_id` to
`invoke_plugin_command`; that dispatcher supplies only declared context keywords.
The bridge declares only `session_id`. No environment/session-history fallback,
model-generated identity or lifecycle hook inference is used.
