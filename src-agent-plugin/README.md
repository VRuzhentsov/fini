# Fini plugins

Agent plugins in the Fini repository. Run the commands on this page from
`src-agent-plugin/`, not from the Fini repository root.

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

## Source location

The shared TypeScript code and the native Hermes and Claude Code plugins are here:

```text
fini/
  src-agent-plugin/
    shared/
    fini-hermes-plugin/
    fini-claude-plugin/
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
  default for both bridges; an explicit shared path is already supported. Final
  storage ownership, the canonical path and any live-storage migration remain
  future decisions, not implemented changes.

This source import keeps the nested npm package and its lockfile separate from
the frontend package. It changes no installed plugin, config or live state.
The standalone checkout and its Git history stay in place for review.

## Runtime and installation

Requires **Node.js 24.12+**, **util-linux `flock` on PATH**, POSIX process groups
and a local filesystem supporting `flock`, atomic rename and directory `fsync`.
Node's native stable type stripping runs `.mts` as ESM: no compiler, Bun,
TypeScript loader, server or service is required at runtime. Bun 1.3.14 and Node
24.13.0 were available during implementation; Node was selected to avoid a
runtime package/FFI dependency for OS locking. Bun execution is not supported or
tested. Do not install only the Hermes subdirectory: the shared sibling is required.

There are **no npm production dependencies**. `typescript`, `@types/node` and
their transitive development dependencies are locked:

```bash
npm ci
npm run check
```

The Python bridge requires the installed Hermes plugin API. Its fixed reviewed
command is `node <resolved-repository>/shared/cli.mts`, not a configurable shell
command. The installed source symlink remains:

```text
~/.hermes/plugins/fini-hermes-plugin -> ../../projects/fini-plugins/fini-hermes-plugin
```

The source import leaves the installed link above in place. A later install
can point a plugin link at `<fini-checkout>/src-agent-plugin/fini-hermes-plugin`.
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
npm run check:ci        # shared and Claude checks; no Hermes install required
npm test                # direct CLI/core, real independent processes and OS lock
npm run test:hermes     # installed Hermes PluginContext/dispatch through Node
npm run test:claude     # native representative hook + relocated cache layout
npm run test:claude:native # optional real Claude CLI ZIP/session-ID acceptance
```

Tests use isolated `HERMES_HOME` directories under `$TMPDIR` (set it to a scratch
location before running outside this workstation). They preserve literal IDs,
multiple sessions/providers, migration, profile isolation, corrupt bytes,
replacement failure, lock contention/owner death, and installed symlink routing.
The Hermes suite requires an installed Hermes runtime on PATH. It registers
this copied plugin through the real PluginContext in temporary homes, then
calls the shared TypeScript code. Its temporary plugin link points at this
source, not at an operator's installed plugin. It changes no live installation.
The CI check runs without Hermes or Claude binaries. The native Claude check
requires the Claude CLI separately; see the [test commands](../docs/testing.md#agent-plugins).

Official sources consulted for this rewrite:

- https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/
- https://nodejs.org/api/typescript.html
- https://raw.githubusercontent.com/util-linux/util-linux/master/sys-utils/flock.1.adoc

Installed Hermes source was also traced: `cli.py` passes `self.session_id` to
`invoke_plugin_command`; that dispatcher supplies only declared context keywords.
The bridge declares only `session_id`. No environment/session-history fallback,
model-generated identity or lifecycle hook inference is used.
