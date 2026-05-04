---
name: fini-scripting
description: "Shared foundation for Fini repo automation architecture. Use when adding or changing Makefile targets, npm scripts, cargo xtask commands, CI command orchestration, release/build/package tooling, or any repo-local automation entry point."
---

# Fini Scripting Foundation

Use this skill when work touches repo automation, command surfaces, package scripts, CI command orchestration, build tooling, packaging tooling, or release tooling implementation.

This is a foundation skill. It defines how automation is structured; domain-specific behavior such as app version policy belongs in the relevant domain skill, such as `fini-versioning`.

## Outcome

Keep automation predictable and discoverable:

- Makefile is the primary human execution entrypoint for repo workflows.
- `npm run` owns JavaScript, TypeScript, frontend, package, and JS/TS test scripts.
- `cargo xtask` owns non-trivial Rust repo automation.
- CI may use `make`, `npm run ...`, or `cargo xtask ...` directly when that is clearer and less duplicative.
- One workflow should have one implementation, reused by local commands and CI when practical.

## Command Surface

Use this default hierarchy:

| Surface | Role |
|---|---|
| `Makefile` | Human-facing entrypoint, memorable workflow names, guard checks, orchestration across tools |
| `npm run` | JS/TS/frontend/package scripts, Vite/Tauri package scripts, JS/TS tests |
| `cargo xtask` | Non-trivial repo automation, file parsing/editing, manifest updates, packaging/release orchestration |
| CI workflow steps | May call `make`, `npm run ...`, or `cargo xtask ...` directly based on setup/cache clarity |

Prefer documenting human commands as `make <target>`.

Use direct `npm run ...` in docs or CI when the task is already a package script and wrapping it in Makefile adds no human-facing value.

Use direct `cargo xtask ...` in CI when a job needs the exact automation command and a Makefile wrapper would obscure cache/setup behavior.

## Makefile Rules

Makefile targets should:

- Provide stable, memorable names for humans.
- Validate required inputs and environment variables.
- Orchestrate existing commands across npm, Cargo, containers, Android tooling, or git.
- Call `npm run` for JS/TS/package tasks when appropriate.
- Call `cargo xtask` for non-trivial automation logic when appropriate.

Keep Makefile recipes simple. Move parsing, file mutation, and complex branching into `xtask` or the domain's native command runner.

## npm run Rules

Use `npm run` for:

- Frontend development and builds.
- Vite and Vue TypeScript checks.
- Tauri CLI package scripts exposed through `package.json`.
- Jest, Playwright, and other JS/TS test scripts.
- Package-manager behavior where `package.json` is the natural owner.

Do not force every `npm run` command behind Makefile. Add a Makefile wrapper when the command becomes a broader repo workflow, needs guard checks, or is intended as a common human entrypoint.

## cargo xtask Rules

Use `cargo xtask` for automation that needs to:

- Parse or rewrite JSON, TOML, lockfiles, manifests, generated metadata, or release files.
- Validate git or release state beyond simple shell checks.
- Share identical logic between local commands and CI.
- Coordinate packaging, release, build metadata, or multi-step repo maintenance.

Keep `xtask` at the repo root:

```text
xtask/
  Cargo.toml
  src/main.rs
```

Run directly when needed:

```bash
cargo run --manifest-path xtask/Cargo.toml -- <command> [args]
```

Prefer Makefile wrappers for human-facing `xtask` workflows.

## Script Policy

Avoid ad hoc JavaScript, TypeScript, shell, PHP, or Python scripts for repo automation unless the repo deliberately adopts that runner for a clear reason.

If adding a new automation runner, make it explicit in project dependencies and update this skill with when to use it.

Shell in Makefile is acceptable for simple guards and orchestration. Use `xtask` once logic becomes stateful, parser-heavy, or reused across local and CI workflows.

## Verification

When changing automation:

- Run the narrowest command that compiles or validates the automation entry point.
- For `xtask`, prefer `cargo check --manifest-path xtask/Cargo.toml`.
- For Makefile target changes, run a safe dry command or a target-specific validation when possible.
- Avoid running release, push, deploy, or destructive targets unless the user explicitly requested that action.
