# fini-hermes-plugin

Explicit local quest/session associations, qualified by provider (`hermes` or
`claude`). This plugin does not call Fini, validate quest/session existence,
read transcripts, modify Fini records, infer an active quest, calculate progress,
or automatically inherit links on fork/compression. Linking is not completion.

## Commands

Once separately enabled in a new Hermes process:

```text
/fini-link <quest-id>
/fini-sessions <quest-id>
/fini-link-claude {"quest_id":"<quest-id>","session_id":"<claude-uuid>"}
```

- `/fini-link` retains its original syntax and uses only the actual current
  session ID supplied by Hermes' slash-command context. There is no session-ID
  argument, model-selected ID, environment fallback, or inferred quest. Missing
  runtime identity returns an error without writing state.
- `/fini-link-claude` accepts exactly two JSON string fields, `quest_id` and
  `session_id`. JSON is used to preserve literal identifiers without shell
  splitting or whitespace normalization. The Claude ID must be a hyphenated UUID
  (8-4-4-4-12 hexadecimal characters); case and every character are retained.
  Format validation is not verification that the Claude session exists. It never
  associates the current Hermes session.
- `/fini-sessions` lists all explicit associations for that exact quest in the
  active profile. An unknown quest returns empty lists.
- All commands return JSON. Listing includes `quest_id`, `sessions` (objects
  with `provider` and `session_id`), legacy `session_ids` (**Hermes only**), and
  `state_path`. Link results additionally include `provider`, `session_id`, and
  `added`. Repeating a provider/ID pair is a no-op (`added: false`). Identical IDs
  under different providers are distinct links; one session can link to multiple
  quests. Errors return `error` without clearing existing state.
- For the original commands, the entire raw argument is still the literal quest
  ID: never stripped, split, converted, repaired, or shell-parsed. Do not add
  shell quotes unless they are part of the ID. Empty/whitespace-only IDs are
  rejected. Frontends may parse outer whitespace before dispatch.

## Assistant-initiated current-session link

Once the plugin is enabled in a **new process** and its `fini` toolset is
available to that session, a request such as “Connect this session to quest
<quest-id>” can use:

```json
{"name":"fini_link_current_session","arguments":{"quest_id":"<quest-id>"}}
```

Only `quest_id` is accepted. Hermes injects the actual caller's `session_id`
as a separate per-invocation keyword, never a model argument. The adapter
forwards it to the same TypeScript command/core used by `/fini-link`, with the
same JSON result and local storage. Missing/empty identity and unexpected fields
fail without writes, even when an inherited environment contains another ID.
No database lookup, model-supplied ID, lifecycle cache, MCP or service is needed.

“Current” means the **calling agent's current session**, not an ancestor or a
stable conversation alias. A delegated child links itself; to link the user's
root conversation, the root assistant must call the tool itself, not delegate
it. Compression can change the agent's session ID; subsequent calls link the
new current ID rather than automatically copying an old association. Concurrent
callers share no captured identity. Native `/fini-link` remains the deterministic
manual alternative. Claude's existing `/fini:link` remains native-hook-owned;
this change adds no Claude assistant-callable tool or MCP.

## Runtime state and migration

By default uses Hermes' `plugins.plugin_storage.plugin_data_dir()` storage convention
(resolved without eager directory creation so TypeScript validates first):

```text
$HERMES_HOME/plugin-data/fini-hermes-plugin/sessions.json
$HERMES_HOME/plugin-data/fini-hermes-plugin/sessions.lock
```

Default: `~/.hermes/plugin-data/fini-hermes-plugin/sessions.json`. Named profiles
use their own active Hermes home; the response's `state_path` identifies scope.
Registration creates no state. Nothing is stored in the source checkout or
Hermes' session database.

Optional `FINI_SESSIONS_PATH` selects an **absolute local file path** explicitly.
Both the Hermes and [native Claude bridge](../fini-claude-plugin/README.md) accept
this same environment variable; use it in both newly launched processes to share
an approved non-default store. Empty/relative values fail core validation with no
fallback. Unset Hermes behavior is unchanged and remains context/profile-local.
No store is moved or merged, and no config/environment setting is changed by this
repository. Claude without an override targets the standard default Hermes home,
not a profile selected in some other running process. Review returned `state_path`.

Version 2 uses explicit provider namespaces:

```json
{
  "version": 2,
  "quests": {
    "<literal quest ID>": {
      "hermes": ["<runtime Hermes session ID>"],
      "claude": ["<explicit Claude UUID>"]
    }
  }
}
```

Either provider may be absent. Version 1 (`quests: {"id": ["session ID"]}`) is
read as Hermes-only. Listing does not rewrite it. The next successful link
(including a duplicate Hermes link) atomically persists v2, preserving every
existing quest and Hermes association. Corrupt/unsupported schemas, providers,
or malformed Claude IDs fail closed. Do not run an older plugin writer after
migration: it cannot read v2. No eager migration or automatic provider detection.

All readers/writers share a stable advisory `flock` sidecar. Writes use a
same-directory temporary file, file `fsync`, atomic replacement, and directory
`fsync`; JSON/lock files are created mode 0600. Concurrent plugin threads and
processes cannot lose each other's updates. The shared TypeScript core uses
util-linux `flock --no-fork` (compatible with the previous Python `fcntl.flock`)
with an 8-second acquisition deadline, never stale-breaks/unlinks the sidecar,
and runs the transaction under that kernel-owned lock. Requires Node.js 24.12+,
util-linux `flock` on PATH, POSIX and a local filesystem with working locks/atomic
rename. Manual edits must respect the lock. See the [root README](../README.md)
for subprocess limits and late-failure/retry semantics.

## Installation and activation

```text
~/.hermes/plugins/fini-hermes-plugin -> ../../projects/fini-plugins/fini-hermes-plugin
```

The link above is the existing installation, preserved for review. The copied
source is now in `src-agent-plugin/` in Fini. To install that source later,
point the link at `<fini-checkout>/src-agent-plugin/fini-hermes-plugin` after
approval. Keep its sibling `shared/` folder available. This import changes
neither the link nor the standalone checkout.

Enabling requires separate approval; when approved:

```bash
hermes -p default plugins enable fini-hermes-plugin
```

Then start a new Hermes process; existing processes do not reload this code.
If the session uses an explicit toolset allowlist, it must also include `fini`
(e.g. through `hermes tools enable fini` after approval). Plugin registration and
toolset visibility are separate from activating the already-running assistant.
No activation/config changes are performed by this implementation or its tests.
No other profile is configured. An explicit manual invocation can persist a
user-approved association without enabling the plugin; this does not activate
its slash commands or hooks.

## Development and verification

- `plugin.yaml`: standalone identity, one `fini` tool, no hooks.
- `__init__.py`: registration, strict native tool envelope, profile/context resolution and bounded subprocess
  JSON transport only; no domain or persistence operations.
- `../shared/*.mts`: all shared business logic, storage, CLI and OS lock runner.
- `tests/test_sessions.py`: isolated real PluginContext/command-dispatch tests.
- `../shared/tests/core.test.mts`: direct CLI and atomic persistence tests.

From this directory, using the installed Hermes runtime:

```bash
hermes --run-module unittest discover -s tests -v
```

Exercise a source symlink in a temporary Hermes home before adding/running
targeted tests. Tests use temporary homes under `$TMPDIR`, explicit fixture
session contexts and real `PluginContext`/`invoke_plugin_command`; no live Fini
records or session links are changed. The test link points at this source,
independent of any live installation. Coverage includes all original behavior,
v1 read-only access and persistence migration, provider-qualified mixed links,
same IDs across providers, deduplication, literal IDs, malformed UUIDs, profile
isolation, corrupt-state preservation, replacement failure, concurrent threads,
and independent writer processes. Doctor checks import/registration, not
interactive frontend acceptance.

## Supported Hermes integration

`ctx.register_command()` handlers may opt into `session_id`, `session_key`,
`platform`, and `source` keyword arguments. The classic CLI supplies its actual
`self.session_id`. No hooks or session-history lookup are needed; lifecycle hooks
cannot choose a user-approved quest and do not cover all resumed sessions.

Installed-source tool provenance (read-only `~/other-projects/hermes-agent`):

- `hermes_cli/plugins_context.py:272-309`: public `ctx.register_tool`, registering
  the native handler without capturing session identity.
- `agent/tool_executor.py:1733-1754`: each call supplies `agent.session_id` to
  `model_tools.handle_function_call`, separately from model arguments.
- `model_tools.py:849-872`: forwards that call's `session_id` to registry dispatch.
- `tools/registry.py:1076-1100`: validates tool arguments, signature-filters runtime
  keyword arguments, then calls the handler with the two channels kept separate.
- `tui_gateway/methods_tools_plugin_commands.py:18-35`: native slash dispatch
  supplies the active session agent's ID (session key before agent creation).
- `tools/delegate_tool_child_run.py:61`: delegated sessions have their own child ID.

The tests execute real `PluginContext` registration and installed
`model_tools.handle_function_call` -> registry -> Python -> TypeScript dispatch,
not a mocked handler/context injector. They check separate concurrent sessions,
conflicting inherited IDs, caller vs conversation/task identifiers, duplicate
links, literal quests, missing identity and attempted argument spoofing. This is
an exercised dispatch-boundary test plus a source-traced executor; no live model
turn or live association is used as an acceptance probe.

Official references:

- https://hermes-agent.nousresearch.com/docs/user-guide/features/plugins/
- https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/#register-slash-commands
- https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/#store-durable-state
