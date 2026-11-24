# Release and versioning

How a Fini release runs, and which version metadata must agree. The secrets
that the release needs and how to add them: `README.md`, "Release". Where CI
builds each target: [`ci.md`](ci.md).

## The release flow

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

## Release tags

- The release pipeline starts on a tag push only (`v*`); a push to `main` does not start release workflows.
- Release tags are annotated and GPG-signed with the configured release key.
- The release flow first pushes the version-bump commit to `origin/main`, then creates and pushes the release tag that points to that exact commit.
- The release tag is the deployment trigger; the tagged commit is the source of truth for package metadata and already contains the release version files.

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

`release-dry-run.yml` (`workflow_dispatch`, `full_matrix`) runs the quality
gates and the full build matrix with no publication.

## Quality Gates

`release-tag.yml` calls `quality-gates.yml`. CI runs these steps in the
`containers/` images with the container engine of CI (`.github/actions/container-stage`; [`container.md`](container.md), "Container engine"):

- backend unit tests in the `linux` `build` image: `cargo test --features cli-plane,ui-plane,desktop-updater`; it compiles the tests first
- CLI runtime image: `containers/linux/Containerfile`, stage `runtime`, and its smoke check (`--help`)
- desktop e2e: `npm run gate:e2e:run` in the `linux` `e2e` image; it builds the e2e binaries, starts the test devices in the container, and runs Playwright

The desktop e2e waits `FINI_E2E_CI_TEST_DEVICE_WAIT_SECS` seconds for the
test devices to start: 180 by default (`scripts/ci-gate.mts`). Do not lower
that time for release evidence. If the test devices do not start, the runner
output and the test device logs are in the `Print E2E artifacts on failure`
step of the Quality Gates job.

`quality-gates.yml` is the one definition of the Quality Gates: `ci.yml` and
the release workflows call it.

A developer can run the same commands locally for an early check (for
example `npm run gate:be-unit`, `npm run gate:e2e:run`; see
[`testing.md`](testing.md)). That local run is not a release step.

## Re-push of a failed release tag

Retargeting or re-pushing an existing release tag is allowed only when no
GitHub release, package artifact, Docker image, Play upload, or other
external release output was published, and only when the user asks for it.
If any release output was published, the existing tag stays as it is, and the
user starts the Release Button again for a new patch release.

## Versioning

Every shipped artifact and visible app version agrees with the release:

- The signed `v*` tag is the deployment trigger.
- The tagged commit is the source of truth for package metadata.
- The release commit must already contain the release version files.
- The app About version must match the package metadata in the tagged source.
- CI should build from the tagged source without inventing a different release version.

### Version surfaces

| Surface | Purpose |
|---|---|
| `package.json` | Frontend/package metadata and current Settings/About source |
| `package-lock.json` | npm lockfile root package version |
| `src-tauri/Cargo.toml` | Rust crate and CLI version metadata |
| `src-tauri/Cargo.lock` | Resolved root crate version |
| `src-tauri/tauri.conf.json` | Tauri app/bundle version metadata |
| `src/views/settings/SettingsView.vue` | About version read path |
| `src-tauri/src/services/cli.service.rs` | CLI version read path through Clap package metadata |
| Android build variables | Android `versionName` and `versionCode` behavior |
| `.github/workflows/release-tag.yml` | Release CI build and packaging behavior |

### Release boundary

The release commit should already contain the release version metadata before the signed tag is pushed. CI may still sync metadata from the tag as a defensive build step, but the committed source should not intentionally lag the release tag.

### Automation boundary

Version metadata mutation belongs in `cargo xtask` because it parses and edits multiple manifest formats.

The Release Button workflow (`.github/workflows/release-button.yml`) calls `cargo xtask release-version <x.y.z>` for the release metadata update; releases run only in CI. The release workflows may call the same `cargo xtask` command directly if they need version verification or metadata sync. The Makefile is for local development only ([`development.md`](development.md), "Repo automation").

### Android versioning

Android local deploys may use git-derived dev metadata for `versionName` (latest reachable tag plus short SHA), but `versionCode` always comes from the project's own metadata (`tauri.properties`). Do not override it, with epoch seconds or anything else: an inflated code makes every other build of the same version, including `make android-dev`, fail to install with `INSTALL_FAILED_VERSION_DOWNGRADE`.

Release Android artifacts should align with the tagged release metadata unless a platform-specific store requirement explicitly requires a separate monotonic code.
