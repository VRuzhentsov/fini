---
name: fini-dev-install
description: "Use this for first-run Fini repository setup before normal development work. Ensures required sibling project context exists, especially cloning and verifying ../fini-wiki from FINI_WIKI_REPO when missing, then records evidence for fini-dev bootstrap."
---

# Fini Dev Install

Set up local prerequisites that Fini agents expect before normal development starts.

This skill is intentionally narrow. It prepares sibling project context, not app dependencies or runtime builds.

## Trigger

Run this skill when `fini-dev` sees that `.fini-dev-install.done` is missing at the Fini repo root.

Run it directly when the user asks to bootstrap, install, or set up the Fini development workspace.

## Outcome

Ensure the sibling wiki repo exists and is usable:

- Expected path: `../fini-wiki/`
- Expected remote: `FINI_WIKI_REPO`, or the GitHub SSH URL for a sibling wiki repository inferred from the current checkout's `origin` remote.
- Required entries: `AGENTS.md`, `_hot.md`, `_index.md`, `raw/`, `pages/`, `tools/`

## Workflow

1. Resolve the current repo root and expected wiki path as `../fini-wiki/`.
2. If `../fini-wiki/` does not exist, clone it with:

   ```bash
   git clone <configured-or-inferred-wiki-repo-url> ../fini-wiki
   ```

3. If `../fini-wiki/` exists, verify it is a git repository.
4. Verify the wiki remote includes the configured or inferred wiki repository URL.
5. Verify required entries exist: `AGENTS.md`, `_hot.md`, `_index.md`, `raw/`, `pages/`, `tools/`.
6. Read `../fini-wiki/AGENTS.md` so the active session has the wiki schema before wiki-dependent work.
7. When all checks pass, create `.fini-dev-install.done` at the Fini repo root.

## Marker Rules

Use `.fini-dev-install.done` as a local per-checkout marker.

Write a short plain-text marker with:

- date/time
- wiki path checked
- remote checked
- required entries checked

The marker is local-only and should be ignored by git.

If any check fails, do not create the marker. Report the failed check and the command or path that needs attention.

## Boundaries

- Do not install app dependencies from this skill.
- Do not run builds, tests, package managers, or Tauri commands from this skill.
- Do not modify files inside `../fini-wiki/` except as a result of the initial `git clone`.
- Do not update wiki pages or raw docs; use `fini-wiki` for wiki work after setup succeeds.

## Evidence

Report the concrete evidence collected:

- Whether `../fini-wiki/` already existed or was cloned.
- The remote verification result.
- The required entries verification result.
- Whether `.fini-dev-install.done` was created.
