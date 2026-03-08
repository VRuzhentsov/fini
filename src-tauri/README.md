# src-tauri/

Rust backend of the Fini app, powered by Tauri 2.0.

## Structure

```
src-tauri/
├── src/
│   ├── lib.rs         # App entry point — registers plugins and commands
│   └── main.rs        # Binary entry point (calls lib::run)
├── gen/
│   └── android/       # Generated Android Studio project (see gen/android/README.md)
├── icons/             # App icons for all platforms
├── capabilities/      # Tauri capability definitions (permission scopes)
├── Cargo.toml         # Rust dependencies
├── build.rs           # Tauri build script
└── tauri.conf.json    # Tauri configuration (app identity, window, plugins, bundle)
```

## Key files

### `tauri.conf.json`

Central config for the app:
- `identifier`: `com.fini.app` — used as the app ID across all platforms
- `plugins.sql.preloadConnections`: SQLite databases to open on startup
- `bundle.targets`: which formats to build (`deb`, `rpm`, `appimage` on Linux; `msi`, `nsis` on Windows)

### `src/lib.rs`

Registers all Tauri plugins and command handlers. Add new Rust commands here via `tauri::generate_handler![]`.

Currently registered plugins:
- `tauri-plugin-opener` — opens URLs/files with the system default app
- `tauri-plugin-sql` (SQLite) — local database access from the frontend

### `capabilities/`

Defines what permissions the frontend has. Each capability file scopes which Tauri APIs and plugins the app can call.

## Platform notes

- **Linux**: Sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup to ensure Wayland compatibility
- **Android**: Built via `npm run tauri android build`; project lives in `gen/android/`
- **Flatpak**: Packaged via `com.fini.app.yml` at the repo root

## Adding a Rust command

1. Define the function in `src/lib.rs` with `#[tauri::command]`
2. Register it in `tauri::generate_handler![your_command]`
3. Call it from the frontend with `invoke("your_command", { ...args })`
