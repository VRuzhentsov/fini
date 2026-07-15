# CLI Binary Contract

Fini exposes a CLI-only binary named `fini` for synchronous automation.

## Entry Points

| Binary | Build features | Contract |
|---|---|---|
| `fini` | `cli-plane` | CLI commands only; no GUI launch behavior |
| `fini-app` | `ui-plane` | Desktop GUI app used by launchers and bundles |
| mobile app | `ui-plane` | Mobile UI only; CLI modules and dependencies are excluded |

`fini app` is not a supported compatibility alias. Desktop launchers must invoke `fini-app` directly.

## Feature Planes

- `cli-plane` owns CLI parsing and CLI-only dependencies such as `clap`.
- `ui-plane` owns the Tauri app runtime and mobile/desktop UI entrypoint.
- Desktop app builds enable only `ui-plane`; local automation uses the separate `fini` CLI binary.
- Docker runtime builds enable only `cli-plane` and expose the CLI binary by default.
- Mobile builds enable only `ui-plane`; adding `cli-plane` to mobile builds violates this contract.

## Release Artifacts

GUI and CLI distribution are separate release surfaces.

| Surface | Artifact shape |
|---|---|
| Linux GUI | `.deb`, `.rpm`, `.AppImage` |
| Windows GUI | NSIS setup `.exe` |
| Linux CLI | `fini-vX.Y.Z-linux-RUST_TARGET-cli.tar.gz` containing `fini` |
| Windows CLI | `fini-vX.Y.Z-windows-RUST_TARGET-cli.zip` containing `fini.exe` |
| Docker runtime | Image entrypoint `/usr/local/bin/fini` |

Desktop installers must not be required to expose the CLI on `PATH`. Users who want CLI-only automation should install the standalone CLI artifact or use the Docker runtime image.

## CLI Updates

`fini update` is the explicit update entrypoint for the standalone CLI. It uses
`self_update` with Rustls transport and an embedded zipsign Ed25519 verification
key; it does not initialize Tauri or use the desktop updater manifest.

CLI updater archives are selected from GitHub Releases by Rust target plus the
`cli` identifier, for example:

```text
fini-vX.Y.Z-linux-x86_64-unknown-linux-gnu-cli.tar.gz
fini-vX.Y.Z-windows-x86_64-pc-windows-msvc-cli.zip
```

Each CLI updater archive is zipsign-signed in the release workflow with the
private `FINI_CLI_ZIPSIGN_PRIVATE_KEY` Actions secret. The standalone CLI embeds
only the corresponding public key at `src-tauri/keys/fini-cli-zipsign.pub`.
This trust root is intentionally separate from the Tauri/Minisign desktop updater
key and `latest.json` manifest.

`fini update --dry-run` discovers the newest matching CLI release without
installing it. Explicit updates bypass the automatic-check interval. Ordinary CLI
invocations perform a quiet automatic update check at most once per 24 hours;
set `FINI_DISABLE_AUTO_UPDATE=1` to disable only that automatic path. Update
failures in the automatic path do not change the normal command result.

On Windows, `self_update` owns the post-exit replacement behavior. Windows
archive/replacement validation runs in GitHub Actions; it cannot be proven by a
Linux-only local test.

## Desktop Updates

Desktop app updates are automatic at startup for release desktop builds compiled
with `ui-plane,desktop-updater`. They use Tauri's signed updater artifacts and a
separate GUI manifest so desktop installers do not share CLI updater targets:

```text
https://github.com/VRuzhentsov/fini/releases/latest/download/latest.json
```

The manifest publishes desktop bundle targets such as `linux-x86_64-appimage`,
`linux-x86_64-deb`, `linux-x86_64-rpm`, and `windows-x86_64-nsis`, with generic
fallbacks for AppImage and NSIS targets. Release builds must provide
`FINI_TAURI_UPDATER_PUBKEY` plus Tauri signing secrets so the app can verify and
install updates when Settings -> Updates -> Automatic updates is enabled.
Turning that setting off skips the next startup auto-update check.
`FINI_DISABLE_AUTO_UPDATE=1` disables the startup check for diagnostics;
`FINI_DESKTOP_UPDATE_ENDPOINT` and `FINI_DESKTOP_UPDATE_PUBKEY` override the
release channel for staging.

## Verification

Use these checks when changing the binary contract:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --bin fini --features cli-plane
cargo build --manifest-path src-tauri/Cargo.toml --bin fini-app --features ui-plane
npm run tauri build -- --no-bundle --features ui-plane
npm run tauri android build -- --features ui-plane --target aarch64
```
