# Fini agent plugin: development

The plugin is the [`fini/`](fini/README.md) folder. That guide gives installation, commands,
tools and rules. This file is for changes to the plugin.

## Layout

```text
src-agent-plugin/
  fini/                  # the plugin: an install copies only this folder
    .claude-plugin/      # Claude Code manifest
    commands/            # /fini:* command files for Claude Code
    hooks/               # Claude Code mod: hooks.json and register.js
    plugin.yaml          # Hermes manifest
    __init__.py          # Hermes commands, tools and pre_llm_call hook
    shared/
      core.mts           # command parsing, Fini CLI checks, execution, quest note
      store.mts          # RelationsRepository on node:sqlite
      cli.mts            # one JSON request on stdin, one result on stdout
      transport.mts      # bounded JSON input and output
  tests/                 # shared core, Hermes and native Claude checks
  package.json           # development tools only
```

The plugin has one version for both agents. Change it in `fini/.claude-plugin/plugin.json`,
`fini/plugin.yaml` and `package.json` together; `tests/shared/version.test.mts` checks that they agree.
Claude Code updates an installed copy when this version changes.

The repository root has `.claude-plugin/marketplace.json`, which lists `./src-agent-plugin/fini`
for Claude Code installs. Hermes installs the same folder from its subfolder path.

Keep `fini/` self-contained. Do not add a `package.json` or npm packages to it: Hermes runs
`npm ci` for a plugin folder that has one, and Claude Code does not install npm packages.
The plugin needs only Node and the `fini` CLI at runtime. Node runs the `.mts` files directly,
with no build step. Both adapters start Node with `--disable-warning=ExperimentalWarning`,
because `node:sqlite` is still marked experimental.

## Adapters

Each adapter gives only the runtime session ID and shows the result. The shared core owns
all rules and storage.

- Claude Code (`hooks/register.js`): a mod. It answers the `/fini:*` command files with
  `command.run` and display-only output, so the command body never reaches the model.
  The mod API cannot register names with a colon, so the command files keep the names.
  It registers the model tools with `$.tool.register` and answers them in `tool.run`.
  The session ID comes from `$.session.id()`. The quest note is added with `$.session.append`,
  because a higher-tier plugin can skip a `prompt.compose` hook. Pass `$` only to functions
  at the top level of the module; the mod loader refuses other forms.
- Hermes (`__init__.py`): registers commands, tools in the `fini` toolset and a `pre_llm_call`
  hook. Each handler gets `session_id` from Hermes at dispatch, also for resumed and
  delegated sessions. No environment variable or transcript gives the current session.

## Transport

`node fini/shared/cli.mts` reads one JSON request:

```json
{"command":"fini:relations","raw_args":"","current":{"agent":"hermes","session_id":"runtime-owned-id"},"db_path":"/absolute/disposable/agent-sessions.db"}
```

- A model tool sends its fields as `input` (`quest_id`, `agent`, `session_id`, `primary`)
  in place of `raw_args`. The core makes the same tokens from them.
- `fini:context` is an internal request for the quest note; it is not a user command.
- `db_path` is optional. This is a local process transport, not a service or MCP server.

Command text uses spaces between tokens. Single or double quotes group a token; a backslash
escapes the next character outside single quotes. IDs keep all characters, case and leading zeros,
must not be blank, have no control characters and do not start with a hyphen. Agent names use
lowercase letters, digits, `_` and `-`, starting with a letter. Claude session IDs must be
hyphenated UUIDs. Unknown, repeated, incomplete and conflicting options return errors.

Results are JSON. Link and unlink return `changed` and the quest's sessions for each agent;
session queries return `quests`. Each result has `db_path`. A handled failure returns `error`.

## Checks

From `src-agent-plugin/`, with `TMPDIR` set to a scratch directory:

```bash
npm ci
npm run check               # typecheck, shared core tests, Hermes tests
npm run test:claude:native  # real Claude Code with a dead model endpoint
claude plugin validate fini
```

- Tests use real Fini CLI records in disposable homes and isolated plugin databases.
- The Hermes tests use the installed Hermes `PluginContext`, `PluginManager` and tool registry.
- The native Claude check loads the `fini/` folder as a ZIP copy, as an install does. It runs the
  commands with real command discovery and mod dispatch, with no model turns and no cost.
- No hosted CI runs these checks.

## References

- https://nodejs.org/api/sqlite.html
- https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/
- https://code.claude.com/docs/en/plugins/mods/api
- https://code.claude.com/docs/en/plugins/mods/events
- https://code.claude.com/docs/en/plugins-reference
