# Fini — Agent Instructions

## Structure

- Frontend (Vue 3): `src/` — see `src/README.md`
- Backend (Rust + Tauri): `src-tauri/` — see `src-tauri/README.md`
- Domain model specs: `spec/`

## Knowledge base

Business and product knowledge lives in the `fini-wiki` wiki at `~/projects/fini-wiki/`. When a task needs business or product context (team info, priorities, metrics, strategic decisions, domain semantics, terminology, or historical intent behind Fini concepts), follow this retrieval protocol:

**Wiki path:** `~/projects/fini-wiki/`

1. **Hot cache first.** Read `_hot.md` first. It contains active threads and key numbers and should resolve most queries.
2. **Master index.** Read `_index.md` if the hot cache is not enough. Check the "Recently Active" section.
3. **Domain sub-index.** Open 1-2 relevant `_index-{domain}.md` files. Never open all sub-indexes at once.
4. **Grep fallback.** Search `wiki/**/*.md` by keyword if the page is not indexed.
5. **Page limit.** Never read more than 5 wiki pages for one query.

When working inside that directory, load its `AGENTS.md` as the authoritative schema; it extends and overrides the global instructions on conflict.

## Workflow

- Do not commit implementation changes until the user has verified they work.
- Docs and spec changes can be committed freely.

## Command preference

- Prefer `Makefile` targets over raw `npm`/`tauri` commands when possible.
- See `Makefile` for available targets.
- Use `/var/tmp` for temporary files and logs; do not use `/tmp`.
- When stopping known dev processes, prefer `pkill -f "<specific-pattern>"` over PID-based `kill`.

## Release tags

- Release pipeline should be triggered by tag push only (`v*`); main pushes should not start release workflows.
- Release tags must be annotated and GPG-signed with the configured release key.
- Create and push release tags only after the target commit is already on `origin/main`.
- Release tag is the GitOps source of truth for release versioning; workflows sync dependent project versions from that tag, so do not pre-bump version files for normal releases.
