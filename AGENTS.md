# Fini — Agent Instructions

## Structure

- Frontend (Vue 3): `src/` — see `src/README.md`
- Backend (Rust + Tauri): `src-tauri/` — see `src-tauri/README.md`
- Domain and feature specs: `specs/`
- Repo automation: `Makefile` + `npm run` + `xtask/` — see `fini-scripting` skill

## Knowledge base

Business and product knowledge lives in the `fini-wiki` wiki at `~/projects/fini-wiki/`. When a task needs business or product context (team info, priorities, metrics, strategic decisions, domain semantics, terminology, or historical intent behind Fini concepts), follow this retrieval protocol:

**Wiki path:** `~/projects/fini-wiki/`

1. **Hot cache first.** Read `_hot.md` first. It contains active threads, current architecture facts, recently changed semantics, and other high-signal context.
2. **Index second.** Read `_index.md` if the hot cache is not enough. `_index.md` is the canonical wiki navigation file.
3. **Targeted page reads.** Open only 1-2 relevant files under `pages/` based on `_hot.md`, `_index.md`, or a targeted search.
4. **Search fallback.** Search `pages/**/*.md` by keyword if the right page is not obvious from `_hot.md` or `_index.md`.
5. **Page limit.** Never read more than 5 wiki pages for one query unless the user explicitly asks for deeper research.

When working inside that directory, load its `AGENTS.md` as the authoritative schema; it extends and overrides the global instructions on conflict.

## Workflow

- Always load the `fini-dev` skill at the start of development work in this repo.
- Use `fini-dev` to choose which repo-local or gstack skill applies, which Makefile target to run, and what evidence is required before reporting success.
- `fini-dev` orchestrates workflow only; use the specialized skill for the actual domain work when it applies.
- Do not commit implementation changes until the user has verified they work.
- Docs and spec changes can be committed freely.

## Command preference

- Prefer `Makefile` targets over raw `npm`/`tauri` commands when possible.
- See `Makefile` for available targets.
- Load `fini-scripting` before adding or changing repo automation scripts, package scripts, release tooling, packaging tooling, CI command orchestration, or build orchestration.
- Treat `Makefile` as the primary human execution entrypoint; use `npm run` for JS/TS package tasks and `xtask/` for non-trivial repo automation logic.
- Load `fini-versioning` before changing package metadata, app version display, CLI version output, Android versioning, release commands, signed tags, or CI release version sync.
- Use `/var/tmp` for temporary files and logs; do not use `/tmp`.
- When stopping known dev processes, prefer `pkill -f "<specific-pattern>"` over PID-based `kill`.

## Release tags

- Release pipeline should be triggered by tag push only (`v*`); main pushes should not start release workflows.
- Release tags must be annotated and GPG-signed with the configured release key.
- Release flow should first push the version-bump commit to `origin/main`, then create and push the release tag that points to that exact commit.
- Release tag is the deployment trigger; the tagged commit is the source of truth for package metadata and must already contain the release version files.
