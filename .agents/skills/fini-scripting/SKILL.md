---
name: fini-scripting
description: "Shared foundation for Fini repo automation architecture. Use when adding or changing Makefile targets, npm scripts, cargo xtask commands, CI command orchestration, release/build/package tooling, or any repo-local automation entry point."
---

# Fini Scripting Foundation

Use this skill when work touches repo automation, command surfaces, package scripts, CI command orchestration, build tooling, packaging tooling, or release tooling implementation.

This is a foundation skill. It defines how automation is structured; domain-specific behavior such as app version policy belongs in the relevant domain skill, such as `fini-versioning`.

## Outcome

Keep automation predictable and discoverable:

- Makefile is the entrypoint for developers on their own computers only (ADR-0013 D9).
- `npm run` owns JavaScript, TypeScript, frontend, package, and JS/TS test scripts, and every CI step.
- `cargo xtask` owns non-trivial Rust repo automation.
- CI calls `npm run ...` (or `cargo xtask ...`) directly. CI never calls `make`.
- One workflow has one implementation: the npm script. The Make target calls the same script.

## Command Surface

Use this default hierarchy:

| Surface | Role |
|---|---|
| `Makefile` | Local developer shortcuts with memorable names; each recipe calls an npm script or a local-only tool (adb, a dev server) |
| `npm run` | JS/TS/frontend/package scripts, Vite/Tauri package scripts, JS/TS tests, and the CI gate steps (`gate:*`, `test:e2e:*`, `build:android:e2e`) |
| `scripts/*.mts` | The logic behind an npm script that is more than one command (`ci-gate.mts`, `e2e-android.mts`, `with-devtools-capability.mts`). Node 24 runs them as they are (type stripping): use only types that can be erased, so no `enum` and no parameter properties. Older scripts are still `.mjs` |
| `cargo xtask` | Non-trivial Rust repo automation, file parsing/editing, manifest updates, packaging/release orchestration |
| CI workflow steps | One `run: npm run <script>` (or `cargo xtask ...`) for each step; no `make` |

Document a command for developers as `make <target>` when a target exists; document the same command for CI as `npm run <script>`.

Playwright runs from npm scripts, not from Make or xtask.

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

Node scripts in `scripts/*.mts` (TypeScript that Node 24 runs as it is) are the adopted runner for the logic behind npm scripts and CI steps (ADR-0013 D9): they need no install beyond Node, which every CI job and developer has. Avoid other ad hoc runners (shell, PHP, Python) for repo automation.

If adding a new automation runner, make it explicit in project dependencies and update this skill with when to use it.

Shell in Makefile is acceptable for simple local guards. Logic that CI also runs goes into `scripts/*.mts` (or `xtask` for Rust-side work), never into a Make recipe.

## Verification

When changing automation:

- Run the narrowest command that compiles or validates the automation entry point.
- For `xtask`, prefer `cargo check --manifest-path xtask/Cargo.toml`.
- For Makefile target changes, run a safe dry command or a target-specific validation when possible.
- Avoid running release, push, deploy, or destructive targets unless the user explicitly requested that action.
