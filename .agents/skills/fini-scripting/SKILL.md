---
name: fini-scripting
description: "Process for changes to Fini repo automation. Use when adding or changing Makefile targets, npm scripts, cargo xtask commands, CI command orchestration, release/build/package tooling, or any repo-local automation entry point."
---

# Fini scripting

Use this skill when work touches repo automation, command surfaces, package
scripts, CI command orchestration, build tooling, packaging tooling, or release
tooling implementation. It adds to `fini-dev`; follow that process first.

The rules (Make is local only, the command surface, the Makefile, npm run and
`cargo xtask` rules, and the script policy) are in `docs/development.md`,
"Repo automation". App version policy is in `docs/release.md`, "Versioning"
(`fini-versioning`).

1. Read `docs/development.md`, "Repo automation", and choose the surface for
   the change from "Command surface".
2. Put logic that CI also runs into an npm script with `scripts/*.mts`, or
   into `cargo xtask`. A Make target only calls it.
3. Document a command for developers as `make <target>` and for CI as
   `npm run <script>`. Update `docs/development.md` and `docs/ci.md` when a
   target or a runner changes.
4. Verify:
   - Run the narrowest command that compiles or validates the automation entry point.
   - For `xtask`, prefer `cargo check --manifest-path xtask/Cargo.toml`.
   - For Makefile target changes, run a safe dry command or a target-specific validation when possible.
   - Avoid running release, push, deploy, or destructive targets unless the user explicitly requested that action.
