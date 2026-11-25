# Fini agent plugin

Links Fini quests to agent sessions in Claude Code and Hermes. One folder is the plugin for both agents.
Fini itself does not know about agents: the plugin reads quests only through the `fini` CLI,
and a link does not change or complete a quest.

## Requirements

- Node.js 24.12 or later, as `node` in `PATH`. The plugin uses Node's built-in `node:sqlite`;
  it has no npm packages and no native modules.
- The `fini` CLI in `PATH`.
- Claude Code 2.1.326 or later, or Hermes.

## Install

Claude Code, at the prompt of a terminal session:

```text
/plugin install fini --marketplace VRuzhentsov/fini
```

Hermes:

```bash
hermes plugins install VRuzhentsov/fini/src-agent-plugin/fini --enable
```

Then start a new session. In Hermes, a running gateway gets the plugin only after it restarts.

## Update

- Claude Code: `claude plugin update fini@fini`, then `/reload-plugins` in an open session.
- Hermes: `hermes plugins update fini-hermes-plugin`. New sessions get the new code;
  restart the gateway for gateway chats.

## Commands

```text
/fini:link <quest-id> [--primary] [--agent NAME --session ID]
/fini:unlink <quest-id> [--agent NAME --session ID]
/fini:relations
/fini:relations --quest ID
/fini:relations --agent NAME --session ID
```

- The default target is the current session. Claude Code or Hermes gives its session ID.
- An explicit target needs both `--agent` and `--session`. Their IDs are not proof that a session exists.
- `relations` without arguments shows the quests of the current session. Each quest has its Fini title
  (null when Fini cannot get the quest) and the sessions of all agents. It does not expand those sessions again.
- `relations --quest ID` shows the sessions of a quest for each agent, with a nullable `primary` and a `related` list.

## Model tools

```text
fini_link       quest_id (required), agent + session_id (both or neither), primary (true/false)
fini_unlink     quest_id (required), agent + session_id (both or neither)
fini_relations  quest_id (optional)
```

- The tools use the same rules as the commands. The runtime gives the current session ID; the model cannot.
- In Claude Code the model calls them as `mcp__fini__fini_link` and so on. In Hermes they are in the `fini` toolset.
- The tools change only links. Quest content changes stay with the `fini` CLI.
- The `fini-sessions` skill in the Fini repository tells the model when to use them.

## Rules

- A link is unique by quest, agent and session. A session can link to many quests,
  and a quest can have many sessions.
- Each quest has at most one primary session for each agent. The first link of an agent becomes primary;
  later links are related.
- A repeated link changes nothing, also not the primary.
- `--primary` (or `primary: true`) makes the target session primary. The former primary stays related.
- An unlink removes only that link. An unlink of the primary leaves no primary; no session takes its place.
- Before a link, the plugin runs `fini quest get --id <ID> --json`. If that fails or returns
  a different ID, no link changes.

## Quest note

The plugin gives the model a note with the quests of the current session and the top active
quests in the Fini list order (`fini quest list`). From `SUGGEST_QUEST_FROM` in `shared/core.mts`,
a session without a quest also tells the model to suggest one quest, one time. Only the user links it.

- Claude Code: the note comes at session start and after each link change. The plugin keeps the last
  note of each session in its plugin store and does not add the same note again on a resume or a reload.
- Hermes: a `pre_llm_call` hook adds the note to each turn. The plugin keeps it for each session
  and reads it again after a link change.

## Storage

The database is `~/.fini/agent-sessions.db`, shared by all agents and Hermes profiles.
`FINI_AGENT_SESSIONS_DB` selects another absolute file. It is separate from Fini's own database.
SQLite owns transactions and concurrency; unique indexes keep links unique and allow one primary
for each quest and agent. The file has mode 0600.

## Limits

- Requests and results are limited to 1 MiB. A call that stops at the timeout can come after a
  committed write: read the relations before you do an operation again.
- The command files have no fallback. If the plugin did not load, their text tells the model to stop,
  but that is not a guarantee. The plugin is not a sandbox, and a runtime session ID is not
  cryptographic proof of identity.
