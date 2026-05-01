---
name: fini-versioning
description: "Shared foundation for Fini app versioning and release metadata. Use when changing package versions, release commands, signed tags, About version display, CLI version output, Android version naming, Tauri/Cargo/npm metadata, or CI release version sync. Depends on fini-scripting when implementation touches Makefile, npm scripts, cargo xtask, or CI command architecture."
---

# Fini Versioning Foundation

Use this skill when work touches app version metadata, release version flow, package manifests, visible app version labels, CLI version output, Android versioning, signed release tags, or CI release version sync.

If the implementation changes Makefile targets, npm scripts, `cargo xtask`, or CI command architecture, also follow `fini-scripting`.

## Outcome

Keep every shipped artifact and visible app version aligned with the release:

- The signed `v*` tag is the deployment trigger.
- The tagged commit is the source of truth for package metadata.
- The release commit must already contain the release version files.
- The app About version must match the package metadata in the tagged source.
- CI should build from the tagged source without inventing a different release version.

## Version Surfaces

Treat these as app version surfaces:

| Surface | Purpose |
|---|---|
| `package.json` | Frontend/package metadata and current Settings/About source |
| `package-lock.json` | npm lockfile root package version |
| `src-tauri/Cargo.toml` | Rust crate and CLI version metadata |
| `src-tauri/Cargo.lock` | Resolved root crate version |
| `src-tauri/tauri.conf.json` | Tauri app/bundle version metadata |
| `src/views/SettingsView.vue` | About version read path |
| `src-tauri/src/services/cli.rs` | CLI version read path through Clap package metadata |
| Android build variables | Android `versionName` and `versionCode` behavior |
| `.github/workflows/release-tag.yml` | Release CI build and packaging behavior |

When changing versioning, trace the write path, persisted metadata, and read path before claiming the version is fixed.

## Release Flow

The human release entrypoint is:

```bash
make release VERSION=x.y.z
```

The release flow should:

1. Require `VERSION` in `x.y.z` form.
2. Require the current branch to be `main`.
3. Require a clean worktree.
4. Require local `HEAD` to match `origin/main` before version mutation.
5. Reject an already-existing `vX.Y.Z` tag.
6. Update all committed version metadata.
7. Verify the release build.
8. Commit version files as `chore: release vX.Y.Z`.
9. Push `main`.
10. Create and verify a GPG-signed annotated `vX.Y.Z` tag on that pushed commit.
11. Push the tag.

Do not keep parallel human release commands. There should be one `make release VERSION=x.y.z` entrypoint.

## Automation Boundary

Version metadata mutation belongs in `cargo xtask` because it parses and edits multiple manifest formats.

Makefile should orchestrate the release and call `cargo xtask` for the metadata update. CI may call the same `cargo xtask` command directly if it needs version verification or metadata sync.

Follow `fini-scripting` for details on balancing Makefile, `npm run`, and `cargo xtask`.

## Android Versioning

Android local deploys may use git-derived dev version metadata, such as latest reachable tag plus short SHA for `versionName` and epoch seconds for `versionCode`, so repeated local installs upgrade cleanly.

Release Android artifacts should align with the tagged release metadata unless a platform-specific store requirement explicitly requires a separate monotonic code.

## Verification

For versioning changes, collect evidence for:

- Write path: command or code that updates version metadata.
- Persisted data: exact files and fields changed.
- Read path: About UI, CLI version, Tauri metadata, or Android metadata as applicable.
- Release safety: branch, clean tree, tag existence, and signed tag behavior when release flow changes.

Use safe checks first. Do not create commits, push branches, or push tags unless the user explicitly requested a real release.
