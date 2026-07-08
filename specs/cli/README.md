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
| Linux CLI | `fini-vX.Y.Z-linux-ARCH-cli.tar.gz` containing `fini` |
| Windows CLI | `fini-vX.Y.Z-windows-ARCH-cli.zip` containing `fini.exe` |
| Docker runtime | Image entrypoint `/usr/local/bin/fini` |

Desktop installers must not be required to expose the CLI on `PATH`. Users who want CLI-only automation should install the standalone CLI artifact or use the Docker runtime image.

## CLI Updates

`fini update` is the manual update entrypoint for the CLI plane. It must use
Tauri's updater package for update discovery, signature verification, download,
and installation rather than scraping GitHub releases or hand-rolling archive
replacement.

The default CLI update endpoint is the signed static manifest published with
GitHub releases:

```text
https://github.com/VRuzhentsov/fini/releases/latest/download/latest-cli.json
```

The CLI updater uses a custom Tauri updater target named
`cli-<platform>-<arch>`, such as `cli-linux-x86_64`, so CLI artifacts do not
share platform keys with GUI app updater bundles. The manifest must contain a
valid SemVer version and a Tauri updater signature for each exposed CLI target.
The initial built-in updater manifest publishes Linux CLI targets only: Tauri's
Windows updater install path expects a Windows installer, not a raw standalone
`fini.exe` replacement.

Release builds should embed the Tauri updater public key with
`FINI_TAURI_UPDATER_PUBKEY`. Local and test builds may supply
`FINI_UPDATE_PUBKEY` at runtime. `FINI_UPDATE_ENDPOINT` may override the default
manifest endpoint for staging channels.

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
