# Fini native Claude Code plugin

Native `commands/*.md` descriptors register `/fini:link`, `/fini:unlink`,
`/fini:primary` and `/fini:relations`. They are commands, not skills.
The [shared guide](../README.md) owns syntax, rules, storage and local checks.

The native `UserPromptExpansion` hook receives the actual runtime `session_id`,
command name and arguments. The hook checks plugin command context, calls the
shared library and returns `decision: block` with the JSON result as its reason.
Both successful results and errors appear in Claude's hook-block message.
This intentionally stops prompt expansion before a model turn.
No model tools, MCP server, stored runtime ID or transcript lookup are involved.

Use a client with native `UserPromptExpansion` support. The local acceptance
check exercises the installed client's actual command discovery and hook dispatch.
It checks all four commands after ZIP cache relocation with twelve invocations,
zero model turns, zero API duration and zero cost. It also checks resume,
explicit targets, both relation directions and invalid input.
Live interactive acceptance remains separate.

## Package and load

From `src-agent-plugin/`:

```bash
npm ci
npm run package:claude
claude plugin validate --strict .artifacts/claude/fini
claude --plugin-dir "$PWD/.artifacts/claude/fini"
```

Load the assembled root, not this source adapter directory. The package copies
canonical shared modules and the locked production dependency tree, including
Drizzle and better-sqlite3's native binaries. Imports stay inside the package.
Cache relocation needs no source checkout, symlinks, network download or install step.
Supported runtime/platform combinations depend on better-sqlite3's included binaries.
Generated output is ignored; rebuild it after changing source.

`source-hashes.json` records the shared source bytes. A local marketplace is also
generated beside the assembled root for separately approved installation.
Packaging and validation do not activate the plugin or change Claude settings.

## Failure limits

The hook bounds input and escaped output to 1 MiB. Stdin requires EOF and has a
15-second deadline. The native hook has a 20-second timeout.
A timeout or output error can follow a committed SQLite transaction;
read relations before repeating an operation.

The command descriptor has no executable fallback. Its text tells the client to
stop if the native hook is unavailable. Disabled hooks, unsupported clients or a
hook launcher failure are not guaranteed to stop model processing. Use only the
validated native path; this plugin is not a model-execution sandbox.
A local process can forge hook JSON. Runtime context is not cryptographic identity proof.

## Checks

```bash
npm run test:claude
npm run test:claude:native
```

Tests isolate HOME, HERMES_HOME, Fini data and the plugin database under `$TMPDIR`.
The native check also isolates Claude settings and supplies a dead local API endpoint.
Python 3 constructs the ZIP; it is needed only for that check.
No credentials or live user conversations are used.
Manifest validation can report separate catalog-submission diagnostics; the
plugin is not prepared for public catalog submission.
