---
name: fini-release
description: "Run and maintain Fini's GitOps release workflow. Use this whenever the user asks to make a release, bump a release version, push a release tag, inspect release readiness, fix release automation, or verify release CI. Releases are initiated by pushing a signed annotated v* tag; before any release push, local agents must collect passing local CI-equivalent evidence for all current CI gates."
---

# Fini Release

Use this skill for operational Fini releases and release workflow changes.

## Outcome

Ship a release through GitOps:

- `main` contains the release version metadata commit.
- A GPG-signed annotated `vX.Y.Z` tag points at that exact commit.
- The tag push starts `.github/workflows/release-tag.yml`.
- CI owns tests, builds, signing readiness, packaging, artifacts, and publication.
- Local release work proves the current release-tag Quality Gates pass before any release metadata commit, branch push, or tag push.

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
8. Run the full local CI release gate below and keep concrete passing evidence before creating the release metadata commit.

If interrupted release metadata is already present, inspect it and decide whether it is intended release metadata before continuing. Re-run the local CI release gate if any release commit, implementation commit, dependency file, workflow, build script, or generated metadata changed after the last successful local gate.

## Local CI Release Gate

Before creating a release metadata commit, pushing `main`, or pushing a release tag for a real release, run every locally reproducible gate from `.github/workflows/ci.yml` and `.github/workflows/release-tag.yml`. Treat this as mandatory release evidence, not optional preflight.

Run these commands from the repo root and capture pass/fail output:

```bash
CONTAINER=docker make pr-gate-fe-unit
CONTAINER=docker make pr-gate-be-compile
CONTAINER=docker make pr-gate-be-unit
npm run build
CONTAINER=docker make runtime-image
CONTAINER=docker make runtime-smoke
CONTAINER=docker make pr-gate-e2e
make android-build-emulator-e2e
```

Run the Snyk gate locally when `SNYK_TOKEN` is available:

```bash
npx snyk test --file=package.json --severity-threshold=high
```

If `SNYK_TOKEN`, Docker/Podman, Android SDK/NDK, KVM/emulator support, or another CI dependency is unavailable, stop before creating the release commit or pushing anything and ask the user how to satisfy the missing gate. Do not mark a release ready with skipped local CI unless the user explicitly changes this release policy for the current release.

The release-tag Quality Gates use `FINI_E2E_CI_ACTOR_WAIT_SECS=180` by default through the Makefile. Do not lower that timeout for release evidence; if actor sockets still do not appear, inspect the container `ps`, `inspect`, and logs output before deciding whether a tag re-push is safe.

The Android emulator workflow's final emulator assertions are not fully represented by `make android-build-emulator-e2e`; when a local emulator is available, locate the debug APK, set `ANDROID_E2E_APK`, and run `bash scripts/android-e2e-assert.sh` against an API 34 x86_64 emulator too. If no local emulator is available, report that the Android emulator runtime assertion is the only remaining local evidence gap and stop for a user decision before push.

## Release Command Policy

The human entrypoint is:

```bash
FINI_RELEASE_LOCAL_CI_PASSED=1 make release VERSION=x.y.z
```

`make release` should:

1. Validate `VERSION` as `x.y.z`.
2. Require branch `main`.
3. Require a clean worktree at command start.
4. Refuse to run unless `FINI_RELEASE_LOCAL_CI_PASSED=1` is set after the local CI release gate passed for the commit being released.
5. Fetch `origin/main` and tags.
6. Require local `HEAD` to match `origin/main`.
7. Reject existing `vX.Y.Z` tags.
8. Update committed version metadata with `cargo xtask release-version`.
9. Commit version files as `chore: release vX.Y.Z`.
10. Push `main`.
11. Create and verify a signed annotated tag.
12. Push the tag.
13. Point the user to the GitHub Actions release workflow.

The release command should not hide the local CI release gate inside version/tag mutation. Run and report the gate explicitly before invoking `make release`, then let tag-triggered CI own package artifact creation and publication after the push.

## Manual Recovery Path

When release metadata already exists because a prior release command was interrupted:

1. Verify only release metadata and intended workflow/doc changes are dirty.
2. Re-run the local CI release gate unless there is already passing evidence for the exact commit that will be pushed.
3. Commit the intended files directly with `chore: release vX.Y.Z` if the user wants them in the release commit.
4. Push `main`.
5. Create and verify the signed annotated tag on the pushed commit.
6. Push the tag.
7. Confirm `.github/workflows/release-tag.yml` started for that tag.

## Failed CI Tag Re-Push

When a release tag exists but the tag-triggered CI failed before completing release artifacts or publication, prefer a safe re-push of the same signed tag object only for transient CI, runner, network, credential, or external-service failures.

Before re-pushing a failed CI tag:

1. Inspect the failed workflow logs and identify the failure cause.
2. Confirm the failure does not require source, version metadata, workflow, dependency, or build-script changes.
3. Verify the local tag signature with `git tag -v vX.Y.Z`.
4. Verify local and remote tag object IDs and peeled commit targets match with `git show-ref --tags vX.Y.Z`, `git rev-parse vX.Y.Z^{}`, and `git ls-remote --tags origin "refs/tags/vX.Y.Z" "refs/tags/vX.Y.Z^{}"`.
5. Confirm the tag target is the intended release commit already present on `origin/main`.
6. Confirm local CI release evidence still applies to that exact commit, or re-run the local CI release gate.

To re-trigger tag CI without changing the release identity, delete only the remote tag and push the existing local signed tag again:

```bash
git push origin :refs/tags/vX.Y.Z
git push origin vX.Y.Z
```

Do not recreate, move, force-push, or retarget an existing release tag unless the user explicitly requests breaking tag immutability for a specific incident. If the failure requires code, metadata, workflow, dependency, or build-script changes, create a new patch release instead of re-pushing the old tag.

## CI Verification

After pushing the tag, collect evidence:

- `git tag -v vX.Y.Z` succeeds locally.
- `git ls-remote --tags origin vX.Y.Z` shows the remote tag.
- `gh run list --workflow release-tag.yml --branch vX.Y.Z` or equivalent shows the release workflow run.
- If `gh` cannot access runs, report the exact tag and workflow file to inspect.

## Reporting

Final release reports should include:

- release version and commit SHA
- local CI release gate commands and outcomes
- pushed branch and tag evidence
- signed tag verification evidence
- CI workflow evidence or the reason it could not be checked
- any local CI release gate that could not run, plus the explicit user decision that allowed continuing despite the gap
