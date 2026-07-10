---
name: fini-release
description: "Run and maintain Fini's GitOps release workflow. Use this whenever the user asks to make a release, bump a release version, push a release tag, inspect release readiness, fix release automation, or verify release CI. Releases are initiated by pushing a signed annotated v* tag; `make release` must run `make pre-release-check` before any version commit, branch push, or tag push."
---

# Fini Release

Use this skill for operational Fini releases and release workflow changes.

## Outcome

Ship a release through GitOps:

- `main` contains the release version metadata commit.
- A GPG-signed annotated `vX.Y.Z` tag points at that exact commit.
- The tag push starts `.github/workflows/release-tag.yml`.
- CI owns tests, builds, signing readiness, packaging, artifacts, and publication.
- Local release work runs the current pre-release check locally before any release metadata commit, branch push, or tag push.

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
8. Use `make release VERSION=x.y.z`; it runs `make pre-release-check` before creating the release metadata commit.

If interrupted release metadata is already present, inspect it and decide whether it is intended release metadata before continuing. Re-run `make pre-release-check` if any implementation commit, dependency file, workflow, build script, or generated metadata changed after the last successful local check.

## Pre-Release Check

Before creating a release metadata commit, pushing `main`, or pushing a release tag for a real release, `make release` must run exactly one local pre-release entrypoint:

```bash
make pre-release-check
```

Treat this as mandatory release behavior, not optional preflight. The command must log its output to `/var/tmp/fini-pre-release/pre-release-check-*.log` by default so the release has a traceable local evidence artifact.

`make pre-release-check` must run the local equivalent of release-tag `Quality Gates` from `.github/workflows/release-tag.yml`. Keep the command list minimal and avoid duplicated expensive work when one target already includes another.

- backend unit tests, using the target that already compiles backend tests
- runtime container image
- runtime smoke check
- E2E actor image compatibility alias, still built for cache/tag parity
- E2E runner image, which owns the Playwright run and spawns actor processes internally
- E2E network/start/wait/run/cleanup, where network/start/wait are compatibility no-ops around the runner-owned actor lifecycle

When changing `make pre-release-check`, verify each command is still necessary against the current Makefile/Dockerfile behavior and update this section if the composition changes.

Security scan, signing readiness, platform package builds, Play upload, Docker publication, and GitHub release publication remain GitHub CI responsibilities and are not part of the local release Quality Gate.

The local gate uses the Makefile's container-engine detection: Docker when available, otherwise Podman. CI can still force `CONTAINER=docker`. If no usable container engine is available, stop before creating the release commit or pushing anything.

If `make pre-release-check` cannot complete because of ENOSPC, missing Docker/Podman, missing dependencies, failed E2E, missing logs, or missing diagnostics, hard stop. Do not commit, push `main`, push a tag, retarget a tag, or ask the user to accept partial evidence.

The release-tag Quality Gates use `FINI_E2E_CI_ACTOR_WAIT_SECS=180` by default through the Makefile. Do not lower that timeout for release evidence. If actor sockets do not appear, use the preserved runner output and actor log files under the mounted results tree to fix the startup failure before any release push.

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
7. Run `make pre-release-check` on the current implementation commit and stop on any failure.
8. Update committed version metadata with `cargo xtask release-version`.
9. Commit version files as `chore: release vX.Y.Z`.
10. Push `main`.
11. Create and verify a signed annotated tag.
12. Push the tag.
13. Point the user to the GitHub Actions release workflow.

Do not manually run `git tag`, `git push origin main`, or `git push origin vX.Y.Z` to bypass `make release`. If `make pre-release-check` fails, fix the failure and rerun `make release` from the beginning.

## Manual Recovery Path

When release metadata already exists because a prior release command was interrupted:

1. Verify only release metadata and intended workflow/doc changes are dirty.
2. Re-run `make pre-release-check` unless there is already passing evidence for the exact implementation commit that will be released.
3. Commit the intended files directly with `chore: release vX.Y.Z` if the user wants them in the release commit.
4. Push `main`.
5. Create and verify the signed annotated tag on the pushed commit.
6. Push the tag.
7. Confirm `.github/workflows/release-tag.yml` started for that tag.

## Failed CI Tag Re-Push

When diagnosing failed release CI, first separate the smallest source-of-truth fix from automation fallbacks. If a release fails because a required public configuration value is absent or malformed, prefer populating that value in the committed release source/config and documenting the matching secret requirement before adding CI-time mutation, normalization, or helper scripts. When there are multiple viable directions, present the options explicitly and recommend the smallest correct one.

When a release tag exists but the tag-triggered CI failed before completing release artifacts or publication, first inspect whether anything was published. Retargeting or re-pushing an existing release tag is allowed only when no GitHub release, package artifact, Docker image, Play upload, or other external release output was published.

Before re-pushing a failed CI tag:

1. Inspect the failed workflow logs and identify the failure cause.
2. Confirm no release output was published.
3. Fix any source, workflow, dependency, or build-script problem on `main`.
4. Run `make pre-release-check` successfully on the fixed implementation commit before retargeting or re-pushing the tag.
5. Verify the local tag signature with `git tag -v vX.Y.Z`.
6. Verify local and remote tag object IDs and peeled commit targets with `git show-ref --tags vX.Y.Z`, `git rev-parse vX.Y.Z^{}`, and `git ls-remote --tags origin "refs/tags/vX.Y.Z" "refs/tags/vX.Y.Z^{}"`.

To re-trigger tag CI after an unpublished failure, delete only the remote tag and push the locally signed tag that points to the fixed `origin/main` commit:

```bash
git push origin :refs/tags/vX.Y.Z
git push origin vX.Y.Z
```

If any release output was published, do not retarget the existing tag. Create a new patch release instead.

## CI Verification

After pushing the tag, collect evidence:

- `git tag -v vX.Y.Z` succeeds locally.
- `git ls-remote --tags origin vX.Y.Z` shows the remote tag.
- `gh run list --workflow release-tag.yml --branch vX.Y.Z` or equivalent shows the release workflow run.
- If `gh` cannot access runs, report the exact tag and workflow file to inspect.

## Reporting

Final release reports should include:

- release version and commit SHA
- `make pre-release-check` command, log path, and outcome
- pushed branch and tag evidence
- signed tag verification evidence
- CI workflow evidence or the reason it could not be checked
- any pre-release check that could not run; this is a blocker unless the user explicitly changes the release policy
