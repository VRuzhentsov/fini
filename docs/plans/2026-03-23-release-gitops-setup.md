# GitOps Release Flow Setup

This document describes the GitHub Actions release flow configured in this repository.

## Workflows

- `.github/workflows/release-dry-run.yml`
  - Runs on every push to `main` and on manual dispatch.
  - Enforces signing-readiness checks.
  - Runs quality gates:
    - `cargo test --manifest-path src-tauri/Cargo.toml`
    - `cargo check --manifest-path src-tauri/Cargo.toml`
    - `npm run build`
  - Builds Linux, Windows, and Android artifacts.

- `.github/workflows/release-tag.yml`
  - Runs on tag push (`v*`).
  - Validates:
    - tag format (`vMAJOR.MINOR.PATCH` or `vMAJOR.MINOR.PATCH-rc.N`)
    - tag actor has maintain/admin permissions
    - tag points to current `origin/main` HEAD
    - tag is signed and annotated
    - matching dry-run succeeded on same commit within 24h
  - Re-runs full gates and platform builds.
  - Publishes release atomically (all platforms must succeed).
  - Stable tags use protected `release` environment approval.
  - RC tags publish as prerelease.

## Required Repository Secrets

- `RELEASE_TAG_GPG_PUBLIC_KEY`
- `COSIGN_PRIVATE_KEY`
- `COSIGN_PASSWORD`
- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`

## Required Repository Configuration

- Protect `main` branch.
- Create protected GitHub Environment: `release`.
  - Add required reviewers.
  - Scope secrets as needed.
- Restrict who can create tags to maintainers.

## Release Procedure

1. Merge release-ready commit to `main`.
2. Confirm `Release Dry Run` passed for that exact commit in last 24h.
3. Create signed annotated tag:

   ```bash
   git tag -s vX.Y.Z -m "vX.Y.Z"
   git push origin vX.Y.Z
   ```

   or prerelease:

   ```bash
   git tag -s vX.Y.Z-rc.N -m "vX.Y.Z-rc.N"
   git push origin vX.Y.Z-rc.N
   ```

4. Approve `release` environment when prompted for stable tags.
5. Verify GitHub Release assets:
   - Linux bundle archive
   - Windows bundle archive
   - Signed Android APK
   - SBOM
   - cosign signatures
   - SHA256 checksums

## Rollback Policy

- Do not rewrite or retag existing versions.
- Mark bad release as deprecated.
- Publish fix-forward hotfix tag (`vX.Y.Z+1` / next patch version).
