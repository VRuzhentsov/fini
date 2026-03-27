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
