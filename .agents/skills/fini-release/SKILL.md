---
name: fini-release
description: "Run and maintain Fini's GitOps release workflow. Use this whenever the user asks to make a release, bump a release version, push a release tag, inspect release readiness, fix release automation, or verify release CI. Releases are initiated by pushing a signed annotated v* tag; local agents must not build app/package artifacts as part of release unless the user explicitly asks for a separate local build."
---

# Fini Release

Use this skill for operational Fini releases and release workflow changes.

## Outcome

Ship a release through GitOps:

- `main` contains the release version metadata commit.
- A GPG-signed annotated `vX.Y.Z` tag points at that exact commit.
- The tag push starts `.github/workflows/release-tag.yml`.
- CI owns tests, builds, signing readiness, packaging, artifacts, and publication.
- Local release work avoids app builds, package builds, Docker builds, and E2E runs unless the user explicitly requests a separate preflight.

## Required Skills

Load these alongside this skill when relevant:

- `fini-dev` first for repo workflow and safety rules.
- `fini-versioning` when changing version metadata surfaces or version invariants.
- `fini-scripting` when changing `Makefile`, `xtask`, package scripts, or CI release workflow structure.
- `fini-release-prep` for Play Store screenshots, listing assets, or marketplace prep.

## Release Preconditions

Before a real release:

1. Confirm the user requested a real release, because the flow pushes `main` and a release tag.
2. Check `git status --short --branch`.
3. Switch to `main` only when the worktree is safe to switch.
4. Pull with `git pull --ff-only origin main`.
5. Confirm `HEAD` matches `origin/main` before creating release metadata.
6. Confirm the target tag does not already exist locally or remotely.
7. Inspect `git diff`, `git diff --cached`, and `git log --oneline --decorate -10` before committing.

If interrupted release metadata is already present, inspect it and decide whether it is intended release metadata before continuing. Do not rerun build-heavy release targets to recover.

## Release Command Policy

The human entrypoint is:

```bash
make release VERSION=x.y.z
```

`make release` should:

1. Validate `VERSION` as `x.y.z`.
2. Require branch `main`.
3. Require a clean worktree at command start.
4. Fetch `origin/main` and tags.
5. Require local `HEAD` to match `origin/main`.
6. Reject existing `vX.Y.Z` tags.
7. Update committed version metadata with `cargo xtask release-version`.
8. Commit version files as `chore: release vX.Y.Z`.
9. Push `main`.
10. Create and verify a signed annotated tag.
11. Push the tag.
12. Point the user to the GitHub Actions release workflow.

It should not run `npm run build`, `npm run tauri build`, `make build`, Docker builds, Android builds, E2E, or package artifact creation. Those belong to CI after tag push.

## Manual Recovery Path

When release metadata already exists because a prior release command was interrupted:

1. Verify only release metadata and intended workflow/doc changes are dirty.
2. Commit the intended files directly with `chore: release vX.Y.Z` if the user wants them in the release commit.
3. Push `main`.
4. Create and verify the signed annotated tag on the pushed commit.
5. Push the tag.
6. Confirm `.github/workflows/release-tag.yml` started for that tag.

## CI Verification

After pushing the tag, collect evidence:

- `git tag -v vX.Y.Z` succeeds locally.
- `git ls-remote --tags origin vX.Y.Z` shows the remote tag.
- `gh run list --workflow release-tag.yml --branch vX.Y.Z` or equivalent shows the release workflow run.
- If `gh` cannot access runs, report the exact tag and workflow file to inspect.

## Reporting

Final release reports should include:

- release version and commit SHA
- pushed branch and tag evidence
- signed tag verification evidence
- CI workflow evidence or the reason it could not be checked
- any local checks intentionally skipped because CI owns them
