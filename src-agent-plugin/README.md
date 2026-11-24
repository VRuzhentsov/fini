# Fini agent plugins

Native Hermes and Claude Code commands share one local TypeScript library.
Fini remains agent-independent. The plugins access quests only through `fini` CLI.
Linking does not change or complete a quest.

## Commands

Both agents use these exact commands:

```text
/fini:link <quest-id> [--agent NAME --session ID]
/fini:unlink <quest-id> [--agent NAME --session ID]
/fini:primary <quest-id> [--agent NAME --session ID]
/fini:primary <quest-id> --clear
/fini:primary <quest-id> --agent NAME --clear
/fini:relations
/fini:relations --quest ID
/fini:relations --agent NAME --session ID
```

- The default target is the actual calling runtime's agent and session ID.
- Explicit targets require both `--agent` and `--session`. Their IDs are not proof that a session exists.
- `primary --clear` needs only the current agent. An explicit `--agent` selects another agent.
- `relations` without arguments shows quests for the current session.
- `relations --quest ID` shows sessions grouped by agent, with a nullable `primary` and a `related` list.
- A link is unique by quest, agent and session. Each session can link to many quests.
- Each quest can have many sessions and at most one primary for each agent.
- The first link in an empty quest-agent group becomes primary. Later links are related.
- A repeated link changes nothing, including the primary selection.
- Promotion requires an existing link. The old primary stays related.
- Unlink removes only the selected link. Removing the primary leaves no primary; no replacement or confirmation follows.
- Clear removes only primary selection. It retains every link.
- Link first runs `fini quest get --id <ID> --json`. CLI failure or a different returned ID changes no links.

Arguments use spaces to separate tokens. Single or double quotes group a token;
backslash escapes the next character outside single quotes. Parsed IDs retain
all characters, case and leading zeros. Shell expansion never runs.
IDs must be nonblank, contain no control characters, and start without a hyphen.
Agent names use lowercase letters, digits, underscores and hyphens, starting with a letter.
Claude session IDs must be exact hyphenated UUIDs. Other agent IDs are opaque text.
Unknown, repeated, incomplete and conflicting options return errors.

Results are JSON. Mutations include the quest's current relation groups;
link and unlink also return `changed`. Session queries return `quests` rows.
Every result includes `db_path`. Handled failures return `error`; callers must inspect it.

## Shared library and storage

```text
shared/
  core.mts       # command codec, CLI quest check, execution and result format
  store.mts      # RelationsRepository interface and Drizzle SQLite implementation
  cli.mts        # one JSON document on stdin, one result on stdout
  transport.mts  # bounded JSON input and output
fini-hermes-plugin/  # native command registration, runtime identity, Python transport
fini-claude-plugin/  # commands/*.md, mod command.run hook, cache packaging
```

The default plugin database is `~/.fini/agent-sessions.db`, shared across agents
and Hermes profiles. `FINI_AGENT_SESSIONS_DB` can select another absolute local file.
This database is separate from Fini's main database and the agents' databases.
SQLite owns transactions and concurrency. Composite and partial unique indexes
protect link and primary uniqueness. Link and promotion use immediate transactions.
Related rows store a null `is_primary`; primary rows store true.
The database file has mode 0600; new parent directories have mode 0700.

The module entry point is `execute(unknown)` in `shared/core.mts`.
The equivalent local transport is `node shared/cli.mts`, with this JSON shape:

```json
{"command":"fini:relations","raw_args":"","current":{"agent":"hermes","session_id":"runtime-owned-id"},"db_path":"/absolute/disposable/agent-sessions.db"}
```

`db_path` is optional. This is a local library transport, not a service or MCP server.
Native bridges own runtime identity. The library owns syntax, rules and storage.
No model-callable tools, skills, session launching, reminders or synchronization are included.

Old JSON files remain untouched. There is no migration, import or automatic fallback.
A future approved cutover starts empty. This source change does not activate either
plugin, change live config, replace the standalone source, or create a live SQLite file.

## Runtime, installation and local checks

Run from `src-agent-plugin/`. Requirements: Node as specified in `package.json`,
`fini` CLI on PATH, and a local filesystem with SQLite locking.
Drizzle uses its stable `better-sqlite3` adapter; exact dependency versions are locked.
There is no server, compiler or TypeScript loader at runtime. Node runs ESM `.mts` files directly.
`skipLibCheck` skips third-party declaration checks: Drizzle's declarations include
unused optional database drivers. Plugin source and tests still receive strict type checks.

```bash
npm ci
npm run check
npm run package:claude
claude plugin validate --strict .artifacts/claude/fini
npm run test:claude:native
```

No Make target or hosted CI check exists for this subtree.
`check` runs typecheck, five shared tests, one Claude package test and three
native Hermes dispatch tests. Tests use real Fini CLI records in disposable homes.
All data paths are isolated under `$TMPDIR`; set it to your scratch directory.
The native Claude check uses twelve invocations with a dead model endpoint,
ZIP cache relocation, resume and exact runtime IDs, with zero model turns.
The package includes canonical shared modules and locked production dependencies,
including SQLite native binaries. A relocated cache needs no source checkout or npm install.
Use a runtime/platform supported by the included better-sqlite3 binaries.
See the [Hermes guide](fini-hermes-plugin/README.md) and [Claude guide](fini-claude-plugin/README.md)
for native loading and limits. Local checks do not establish live interactive acceptance.

## Primary references

- https://orm.drizzle.team/docs/get-started-sqlite
- https://github.com/WiseLibs/better-sqlite3
- https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/
- https://code.claude.com/docs/en/plugins/mods/api
- https://code.claude.com/docs/en/plugins/mods/events
- https://code.claude.com/docs/en/plugins-reference
