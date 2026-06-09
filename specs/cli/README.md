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

## Verification

Use these checks when changing the binary contract:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --bin fini --features cli-plane
cargo build --manifest-path src-tauri/Cargo.toml --bin fini-app --features ui-plane
npm run tauri build -- --no-bundle --features ui-plane
npm run tauri android build -- --features ui-plane --target aarch64
```
