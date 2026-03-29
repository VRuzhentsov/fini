# Fini — Agent Instructions

## Structure

- Frontend (Vue 3): `src/` — see `src/README.md`
- Backend (Rust + Tauri): `src-tauri/` — see `src-tauri/README.md`
- Domain model specs: `spec/`

## Workflow

- Do not commit implementation changes until the user has verified they work.
- Docs and spec changes can be committed freely.

## Command preference

- Prefer `Makefile` targets over raw `npm`/`tauri` commands when possible.
- See `Makefile` for available targets.

## Release tags

- Release pipeline should be triggered by tag push only (`v*`); main pushes should not start release workflows.
- Release tags must be annotated and GPG-signed with the configured release key.
- Create and push release tags only after the target commit is already on `origin/main`.
- Release tag is the GitOps source of truth for release versioning; workflows sync dependent project versions from that tag, so do not pre-bump version files for normal releases.
