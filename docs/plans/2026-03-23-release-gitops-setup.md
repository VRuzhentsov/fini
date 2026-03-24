# GitOps Release Flow Setup

This document describes the GitHub Actions release flow configured in this repository.

## Workflows

- `.github/workflows/release-dry-run.yml`
  - Runs on every push to `main` and on manual dispatch.
  - Enforces signing-readiness checks.
  - Validates keyless cosign signing through GitHub OIDC.
  - Runs quality gates:
    - `cargo test --manifest-path src-tauri/Cargo.toml`
    - `cargo check --manifest-path src-tauri/Cargo.toml`
    - `npm run build`
  - Push mode (`main`): runs fast checks (quality gates + Docker cache build).
  - Manual mode (`workflow_dispatch`, `full_matrix=true`): also builds Linux, Windows, and Android artifacts.
  - Android prep build is optimized to a single `aarch64` APK target.
  - Builds Docker image cache for `ghcr.io/<owner>/fini`.

- `.github/workflows/release-tag.yml`
  - Runs on tag push (`v*`).
  - Validates:
    - tag format (`vMAJOR.MINOR.PATCH` or `vMAJOR.MINOR.PATCH-rc.N`)
    - tag actor has maintain/admin permissions
    - tag points to current `origin/main` HEAD
    - tag is signed and annotated
    - matching manual release prep check (`workflow_dispatch`) succeeded on same commit within 24h
  - Re-runs full gates and platform builds.
  - Android release build is optimized to a single `aarch64` APK target.
  - Automatically propagates tag version (`vX.Y.Z` / `vX.Y.Z-rc.N`) into:
    - `package.json`
    - `package-lock.json`
    - `src-tauri/Cargo.toml`
    - `src-tauri/Cargo.lock`
    - `src-tauri/tauri.conf.json`
  - Builds and publishes Docker image to GHCR (`ghcr.io/<owner>/fini:<tag>`).
  - Publishes release atomically (all platforms must succeed).
  - Stable tags use protected `release` environment approval.
  - RC tags publish as prerelease.

## Required Repository Secrets

- `RELEASE_TAG_GPG_PUBLIC_KEY`
- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`

## Cosign Signing Mode

- Artifact signatures use keyless cosign (GitHub OIDC).
- No long-lived cosign private key is stored in repository secrets.

## Required Repository Configuration

- Protect `main` branch.
- Create protected GitHub Environment: `release`.
  - Add required reviewers.
  - Scope secrets as needed.
- Restrict who can create tags to maintainers.

## Release Procedure

1. Merge release-ready commit to `main`.
2. Trigger `Release Prep Check` manually with `full_matrix=true` on that exact commit and confirm it passed within 24h.
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
   - cosign signatures and certificates
   - SHA256 checksums
6. Verify container transport:
   - `ghcr.io/<owner>/fini:<tag>` exists
   - `latest` tag exists for stable releases
   - image digest signature and attestation are present

## Rollback Policy

- Do not rewrite or retag existing versions.
- Mark bad release as deprecated.
- Publish fix-forward hotfix tag (`vX.Y.Z+1` / next patch version).
