---
name: fini-release
description: "Run and maintain Fini's GitOps release workflow. Use this whenever the user asks to make a release, bump a release version, push a release tag, inspect release readiness, fix release automation, or verify release CI. Releases run only in CI: the user starts the Release Button GitHub Action, which commits the version on main and pushes a signed annotated v* tag; the tag push starts release-tag.yml."
---

# Fini Release

Use this skill for operational Fini releases and release workflow changes.

## Outcome

A release runs only in CI:

```
Release Button (.github/workflows/release-button.yml), started by the user
├── version commit `chore: release vX.Y.Z` on main (cargo xtask release-version)
├── trace branch release/vX.Y.Z
└── GPG-signed annotated tag vX.Y.Z on that commit
    └── tag push → .github/workflows/release-tag.yml
        ├── validate-tag, signing-readiness
        ├── quality-gates (quality-gates.yml), android-e2e (not blocking)
        ├── build-linux, build-windows, build-android, build-docker-image
        └── publish-android-internal, publish-stable or publish-prerelease
```

- The user starts the Release Button. An agent does not start it, and does
  not make the version commit or the tag by hand.
- CI owns the quality gates, the builds, the signing, the packages and the
  publication.
- The Makefile is for local development only. No `make` target is a release
  step or a release gate.

## Required Skills

Load these alongside this skill when relevant:

- `fini-dev` first for repo workflow and safety rules.
- `fini-versioning` when changing version metadata surfaces or version invariants.
- `fini-scripting` when changing `xtask`, package scripts, or CI release workflow structure.
- `fini-release-prep` for Play Store screenshots, listing assets, or marketplace prep.

## The Release Button

`.github/workflows/release-button.yml` (`workflow_dispatch`) does these steps
on `main`:

1. It compares the latest `v*` tag with `origin/main`. With no new commits, it
   stops as a skipped run: no commit, no tag, no release run.
2. It chooses the next version from the Conventional Commit subjects (major,
   minor or patch).
3. It runs `cargo xtask release-version <x.y.z>`, commits the version files
   as `chore: release vX.Y.Z`, and pushes `main` and `release/vX.Y.Z`.
4. It makes the GPG-signed annotated tag `vX.Y.Z`, checks it with
   `git tag -v`, and pushes it. The tag push starts `release-tag.yml`.

The repository secrets `RELEASE_AUTOMATION_TOKEN`,
`RELEASE_TAG_GPG_PRIVATE_KEY` and `RELEASE_TAG_GPG_PASSPHRASE` must exist
(see `README.md`, "Release").

## Release readiness

Before the user starts the Release Button, an agent can report the state:

- `git log --oneline "$(git describe --tags --abbrev=0 --match 'v[0-9]*' origin/main)..origin/main"`
  shows the commits that the release will contain.
- `gh run list --workflow ci.yml --branch main --limit 5` shows that CI on
  `main` is green.
- `gh run list --workflow release-dry-run.yml --limit 5` shows the last dry
  run. `release-dry-run.yml` (`workflow_dispatch`, `full_matrix`) runs the
  quality gates and the full build matrix with no publication.

## Quality Gates

`release-tag.yml` calls `quality-gates.yml`. CI runs these steps in the
`containers/` images with the container engine of CI (`.github/actions/container-stage`; `docs/container.md`, "Container engine"):

- backend unit tests in the `linux` `build` image: `cargo test --features cli-plane,ui-plane,desktop-updater`; it compiles the tests first
- CLI runtime image: `containers/linux/Containerfile`, stage `runtime`, and its smoke check (`--help`)
- desktop e2e: `npm run gate:e2e:run` in the `linux` `e2e` image; it builds the e2e binaries, starts the test devices in the container, and runs Playwright

The desktop e2e waits `FINI_E2E_CI_TEST_DEVICE_WAIT_SECS` seconds for the
test devices to start: 180 by default (`scripts/ci-gate.mts`). Do not lower
that time for release evidence. If the test devices do not start, read the
runner output and the test device logs in the `Print E2E artifacts on
failure` step of the Quality Gates job, and fix the startup failure.

When you change the Quality Gates, keep `quality-gates.yml` as the one
definition: `ci.yml` and the release workflows call it.

A developer can run the same commands locally for an early check (for
example `npm run gate:be-unit`, `npm run gate:e2e:run`; see `fini-test`).
That local run is not a release step.

## Failed CI Tag Re-Push

When a release tag exists but the tag-triggered CI failed before completing release artifacts or publication, first inspect whether anything was published. Retargeting or re-pushing an existing release tag is allowed only when no GitHub release, package artifact, Docker image, Play upload, or other external release output was published, and only when the user asks for it.

Before re-pushing a failed CI tag:

1. Inspect the failed workflow logs (`gh run view <run-id> --log-failed`) and identify the failure cause.
2. Confirm no release output was published.
3. Fix any source, workflow, dependency, or build-script problem on `main` through a pull request, and get CI green on it.
4. Verify the tag signature with `git tag -v vX.Y.Z`.
5. Verify local and remote tag object IDs and peeled commit targets with `git show-ref --tags vX.Y.Z`, `git rev-parse vX.Y.Z^{}`, and `git ls-remote --tags origin "refs/tags/vX.Y.Z" "refs/tags/vX.Y.Z^{}"`.

If any release output was published, do not retarget the existing tag. The
user starts the Release Button again for a new patch release.

## CI Verification

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
