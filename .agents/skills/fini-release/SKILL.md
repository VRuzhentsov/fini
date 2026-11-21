---
name: fini-release
description: "Run and maintain Fini's GitOps release workflow. Use this whenever the user asks to make a release, bump a release version, push a release tag, inspect release readiness, fix release automation, or verify release CI. Releases run only in CI: the user starts the Release Button GitHub Action, which commits the version on main and pushes a signed annotated v* tag; the tag push starts release-tag.yml."
---

# Fini release

Use this skill for operational Fini releases and release workflow changes. The
release flow, the Release Button, the release tag rules and the Quality Gates
are in `docs/release.md`.

Load these alongside this skill when relevant:

- `fini-dev` first for repo workflow and safety rules.
- `fini-versioning` when changing version metadata surfaces or version invariants.
- `fini-scripting` when changing `xtask`, package scripts, or CI release workflow structure.
- `fini-release-prep` for Play Store screenshots, listing assets, or marketplace prep.

The user starts the Release Button. Do not start it, and do not make the
version commit or the tag by hand.

## Release readiness

Before the user starts the Release Button, report the state:

- `git log --oneline "$(git describe --tags --abbrev=0 --match 'v[0-9]*' origin/main)..origin/main"`
  shows the commits that the release will contain.
- `gh run list --workflow ci.yml --branch main --limit 5` shows that CI on
  `main` is green.
- `gh run list --workflow release-dry-run.yml --limit 5` shows the last dry
  run.

## Quality Gates changes

- Keep `quality-gates.yml` as the one definition: `ci.yml` and the release workflows call it.
- If the test devices do not start, read the runner output and the test
  device logs in the `Print E2E artifacts on failure` step of the Quality
  Gates job, and fix the startup failure. Do not lower
  `FINI_E2E_CI_TEST_DEVICE_WAIT_SECS` for release evidence.

## Failed CI tag re-push

When a release tag exists but the tag-triggered CI failed before completing
release artifacts or publication, first inspect whether anything was
published (`docs/release.md`, "Re-push of a failed release tag"). Before
re-pushing a failed CI tag:

1. Inspect the failed workflow logs (`gh run view <run-id> --log-failed`) and identify the failure cause.
2. Confirm no release output was published.
3. Fix any source, workflow, dependency, or build-script problem on `main` through a pull request, and get CI green on it.
4. Verify the tag signature with `git tag -v vX.Y.Z`.
5. Verify local and remote tag object IDs and peeled commit targets with `git show-ref --tags vX.Y.Z`, `git rev-parse vX.Y.Z^{}`, and `git ls-remote --tags origin "refs/tags/vX.Y.Z" "refs/tags/vX.Y.Z^{}"`.

If any release output was published, do not retarget the existing tag. The
user starts the Release Button again for a new patch release.

## CI verification

After the Release Button run, collect evidence:

- `gh run list --workflow release-button.yml --limit 3` and `gh run view <run-id>` show the Release Button run and its summary (the planned tag, or the skip).
- `git ls-remote --tags origin vX.Y.Z` shows the remote tag; `git tag -v vX.Y.Z` succeeds after a fetch.
- `gh run list --workflow release-tag.yml --branch vX.Y.Z` shows the release run; `gh run view <run-id>` shows each job (Quality Gates, the builds, the publication).
- If `gh` cannot access runs, report the exact tag and workflow file to inspect.

## Reporting

Final release reports should include:

- release version and commit SHA
- the Release Button run, and the `release-tag.yml` run with the result of each job
- the tag and its signature check
- any job that failed, with the failure cause from its log
