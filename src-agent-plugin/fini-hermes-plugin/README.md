# Fini native Hermes plugin

`register(ctx)` registers four native commands: `/fini:link`, `/fini:unlink`,
`/fini:primary` and `/fini:relations`. There are no model-callable tools or hooks.
The [shared guide](../README.md) owns command syntax, rules, storage and local checks.

The handler accepts the invocation's `session_id`. Hermes supplies the current
session at dispatch, including resumed or delegated sessions. A child identifies
itself, not its parent. No environment ID, transcript search, registration-time
identity or newest-session lookup supplies the current target.
Missing runtime identity returns an error unless an explicit target was supplied.

Python owns registration and bounded JSON subprocess transport only.
The resolved sibling `shared/cli.mts` owns all behavior and persistence.
Transport limits are 1 MiB for input and aggregate output, with a 15-second
process deadline. SQLite and Fini CLI each have an 8-second wait limit.
A transport error can follow a committed write. Read relations before retrying.

## Quest note

A `pre_llm_call` hook adds the quest note to each turn. The plugin keeps the note
for each session and reads it again after a Fini command.

## Installation boundary

Source-only delivery leaves the current installed source unchanged.
After separate activation approval, install dependencies from `src-agent-plugin/`
and point the Hermes plugin link at `<fini-checkout>/src-agent-plugin/fini-hermes-plugin`.
Keep sibling `shared/` and `node_modules/` available; a lone adapter copy is insufficient.
Enable the plugin with the native plugin command, then start a new Hermes process.
Existing processes do not load source changes automatically.

## Local native checks

Run from `src-agent-plugin/`:

```bash
npm run test:hermes
```

The tests use the installed `PluginContext`, `PluginManager` and
`invoke_plugin_command`. They exercise the installed source symlink, colon-qualified
commands, per-call identity, both relation directions, primary removal,
explicit targets, shared profiles and concurrent dispatch in disposable homes.
They also verify that the removed `fini_link_current_session` tool is absent.
A plugin doctor check is separate from dispatch and live frontend acceptance.
No live session, plugin link, configuration or associations change during these tests.
