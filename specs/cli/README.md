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

## Verification

Use these checks when changing the binary contract:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --bin fini --features cli-plane
cargo build --manifest-path src-tauri/Cargo.toml --bin fini-app --features ui-plane
npm run tauri build -- --no-bundle --features ui-plane
npm run tauri android build -- --features ui-plane --target aarch64
```
