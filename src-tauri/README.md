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

See `spec/` at the repo root for domain model specs ([[Quest]], [[spec/Space]], [[RepeatRule]], [[QuestSeries]], [[QuestOccurrence]], [[Reminder]]).

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
| `create_quest` | `{ space_id?, title, description?, priority?, due?, due_time?, repeat_rule?, order_rank? }` (`space_id` omitted -> `"1"`) | `Quest` |
| `update_quest` | `id`, `{ space_id?, title?, description?, status?, priority?, due?, due_time?, repeat_rule?, order_rank?, set_main_at?, reminder_triggered_at? }` (`space_id` updates are non-null) | `Quest` |
| `delete_quest` | `id`                                                         | —            |

Notes:
- Quest ids are UUIDs
- Space ids are strings (`"1"`, `"2"`, `"3"`, or UUID)

## Platform notes

- **Linux**: Sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup to ensure Wayland compatibility
- **Android**: Built via `npm run tauri android build`; project lives in `gen/android/`
- **Flatpak**: Packaged via `com.fini.app.yml` at the repo root

## Postponed

- **Voice / ASR** (`src/voice.rs`, `src/model_download.rs`) — offline speech recognition via sherpa-onnx. Code is present but not compiled or registered. Will be revisited after the core quest flow is stable.
