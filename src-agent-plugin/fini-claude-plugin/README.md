# Fini native Claude Code plugin

Native `commands/*.md` descriptors register `/fini:link`, `/fini:unlink`,
`/fini:primary` and `/fini:relations`. They are commands, not skills.
The [shared guide](../README.md) owns syntax, rules, storage and local checks.

`hooks/register.js` is a Claude Code mod module (`hooks.json` names it under
`modules`). It hooks `command.run` for the four commands. It reads the runtime
session ID with `$.session.id()`, sends one request to `shared/cli.mts` with
`$.process.run`, and returns the JSON result as `{ text, isTextDisplayOnly: true }`.
It does not call `next`, so the command body never reaches the model.
The result shows as normal command output, not as a hook error.
No model tools, MCP server, stored runtime ID or transcript lookup are involved.

The mod API cannot register `fini:*` names: `$.command.register` accepts only
letters, digits, `_` and `-`. The markdown descriptors keep the names, and the
mod answers them. Display-only output needs Claude Code 2.1.326 or later.

The local acceptance check uses the installed client's real command discovery
and mod dispatch. It checks all four commands after ZIP cache relocation with
twelve invocations, zero model turns, zero API duration and zero cost. It also
checks resume, explicit targets, both relation directions and invalid input.
Live interactive acceptance is a separate step. After a source change, run
`/reload-plugins` in the open session.

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

The shared transport limits each request and response to 1 MiB. The mod stops
the core process after 20 seconds and shows an error. A timeout can occur after
a committed SQLite transaction; read relations before you do an operation again.

The command descriptor has no executable fallback. Its text tells the model to
stop if the mod hook did not run. A disabled plugin, an unsupported client or a
failed mod load does not guarantee that the model stops.
Use only the validated native path; this plugin is not a model-execution sandbox.
Runtime context is not cryptographic identity proof.

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
