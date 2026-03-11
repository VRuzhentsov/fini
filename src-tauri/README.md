# src-tauri/

Rust backend of the Fini app, powered by Tauri 2.0.

## Structure

```
src-tauri/
├── src/
│   ├── lib.rs         # App entry point — DB setup, models, command handlers
│   ├── schema.rs      # Diesel table definitions
│   └── main.rs        # Binary entry point (calls lib::run)
├── migrations/        # SQL migrations (Diesel format)
├── gen/
│   └── android/       # Generated Android Studio project
├── icons/             # App icons for all platforms
├── capabilities/      # Tauri capability definitions (permission scopes)
├── Cargo.toml         # Rust dependencies
├── build.rs           # Tauri build script
└── tauri.conf.json    # Tauri configuration (app identity, window, bundle)
```

## Data model

### Space

A named context for grouping quests (e.g. Personal, Work, Side Project).

| Field        | Type    | Notes                        |
|--------------|---------|------------------------------|
| `id`         | i64     | Auto-increment PK            |
| `name`       | String  |                              |
| `item_order` | i64     | Display order, set by client |
| `created_at` | String  | UTC datetime                 |

### Quest

A task. Belongs to an optional Space.

| Field            | Type         | Notes                               |
|------------------|--------------|-------------------------------------|
| `id`             | i64          | Auto-increment PK                   |
| `space_id`       | Option<i64>  | FK → spaces, nullable               |
| `title`          | String       |                                     |
| `description`    | Option<String> |                                   |
| `status`         | String       | `active` \| `completed` \| `abandoned` |
| `priority`       | i64          | 1 = none, 2 = low, 3 = med, 4 = urgent |
| `pinned`         | bool         |                                     |
| `due`            | Option<String> | ISO date string                   |
| `energy_required`| Option<i64>  | Subjective effort level             |
| `completed_at`   | Option<String> | Set automatically on completion   |
| `created_at`     | String       |                                     |
| `updated_at`     | String       |                                     |

## Commands

### Spaces
| Command        | Input                          | Returns       |
|----------------|--------------------------------|---------------|
| `get_spaces`   | —                              | `Vec<Space>`  |
| `create_space` | `{ name, item_order }`         | `Space`       |
| `update_space` | `id`, `{ name?, item_order? }` | `Space`       |
| `delete_space` | `id`                           | —             |

### Quests
| Command        | Input                                                        | Returns      |
|----------------|--------------------------------------------------------------|--------------|
| `get_quests`   | —                                                            | `Vec<Quest>` |
| `create_quest` | `{ space_id?, title, description?, priority?, due?, energy_required? }` | `Quest` |
| `update_quest` | `id`, `{ space_id?, title?, description?, status?, priority?, pinned?, due?, energy_required? }` | `Quest` |
| `delete_quest` | `id`                                                         | —            |

## Platform notes

- **Linux**: Sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup to ensure Wayland compatibility
- **Android**: Built via `npm run tauri android build`; project lives in `gen/android/`
- **Flatpak**: Packaged via `com.fini.app.yml` at the repo root

## Postponed

- **Voice / ASR** (`src/voice.rs`, `src/model_download.rs`) — offline speech recognition via sherpa-onnx. Code is present but not compiled or registered. Will be revisited after the core quest flow is stable.
