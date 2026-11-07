# Fini — Agent Instructions

## Structure

- Frontend (Vue 3): `src/` — see `src/README.md`
- Backend (Rust + Tauri): `src-tauri/` — see `src-tauri/README.md`
- Domain and feature specs: `specs/`
- Repo automation: `Makefile` + `npm run` + `xtask/` — see `fini-scripting` skill

## Workflow

- Always load the `fini-dev` skill at the start of development work in this repo.
- Use `fini-dev` to choose which repo-local or gstack skill applies, which Makefile target to run, and what evidence is required before reporting success.
- `fini-dev` orchestrates workflow only; use the specialized skill for the actual domain work when it applies.
- Write all code names, comments, messages, docs, commits and PR text in STE (`docs/naming.md`, words from `docs/glossary.md`). Check each diff for STE before you commit and before you report a PR as ready (`fini-dev`, "Use STE"). Do not wait for the user to ask for this check.
- Use the pull request as the normal user review and verification surface for ticket/worktree implementation. After local checks pass, commit and push implementation changes to the active PR branch so they are reviewable there; do not leave verified implementation only in local working-tree state unless the user explicitly asks for local-only work.
- If the user explicitly asks to manually verify a local build before review, pause before committing implementation changes and report the exact local state and verification command.
- Docs, specs, and workflow-instruction changes can be committed freely.

## Asking the user for a decision

- Explain before asking. Define every term the decision depends on in plain words ("what it is"), as if the user has not met it; link or add it to `docs/glossary.md`.
- Present each decision as options, each with a concrete example (code or a real scenario from the project) and its tradeoffs spelled out in full sentences, not three-word labels.
- Prefer solutions that already exist in established projects. For each option, name the project or library that does it that way and show how; propose a home-grown design only when no existing one fits, and say why.
- Separate a question from a statement: say explicitly whether you are asking the user to choose or reporting what you have decided.
- When you propose or decide something yourself, still show the options, examples and tradeoffs behind it, so the user can check the choice.

## Command preference

- On a developer's computer, prefer `Makefile` targets over raw `npm`/`tauri` commands when possible. See `Makefile` for available targets.
- In CI, call `npm run` scripts. CI never calls `make`; each Make target calls the same npm script (ADR-0013 D9).
- Load `fini-scripting` before adding or changing repo automation scripts, package scripts, release tooling, packaging tooling, CI command orchestration, or build orchestration.
- Use `npm run` (with logic in `scripts/*.mts`) for JS/TS package tasks and CI steps, and `xtask/` for non-trivial Rust-side repo automation.
- Load `fini-release` before running or changing release commands, signed tags, or release CI verification. Load `fini-versioning` before changing package metadata, app version display, CLI version output, Android versioning, or CI release version sync.
- When stopping known dev processes, prefer `pkill -f "<specific-pattern>"` over PID-based `kill`.

## Release tags

- Release pipeline should be triggered by tag push only (`v*`); main pushes should not start release workflows.
- Release tags must be annotated and GPG-signed with the configured release key.
- Release flow should first push the version-bump commit to `origin/main`, then create and push the release tag that points to that exact commit.
- Release tag is the deployment trigger; the tagged commit is the source of truth for package metadata and must already contain the release version files.
