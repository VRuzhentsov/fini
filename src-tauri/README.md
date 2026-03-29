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

See `spec/` at the repo root for domain model specs ([[Quest]], [[Space]], [[RepeatRule]], [[QuestSeries]], [[QuestOccurrence]], [[Reminder]], [[FocusHistory]], [[DeviceConnection]], [[SpaceSync]], [[Network]]).

## Commands (target naming for upcoming sync implementation)

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
| `update_quest` | `id`, `{ space_id?, title?, description?, status?, priority?, due?, due_time?, repeat_rule?, order_rank? }` (`space_id` updates are non-null) | `Quest` |
| `delete_quest` | `id`                                                         | —            |

### Focus
| Command | Input | Returns |
|---|---|---|
| `get_active_focus` | — | `Quest | null` |
| `set_focus` | `{ quest_id, trigger }` | `FocusHistory` |
| `list_focus_history` | `{ limit?, before? }` | `Vec<FocusHistory>` |

### Device connection (UDP control-plane)
| Command | Input | Returns |
|---|---|---|
| `device_connection_get_identity` | — | `{ device_id, hostname }` |
| `device_connection_enter_add_mode` | — | `()` |
| `device_connection_leave_add_mode` | — | `()` |
| `device_connection_discovery_snapshot` | — | `Vec<{ device_id, hostname, addr, ws_port, last_seen_at }>` |
| `device_connection_presence_snapshot` | — | `Vec<{ device_id, hostname, addr, last_seen_at }>` |
| `device_connection_send_pair_request` | `{ request_id, to_device_id, to_addr }` | `()` |
| `device_connection_pair_incoming_requests` | — | `Vec<IncomingPairRequest>` |
| `device_connection_pair_accept_request` | `{ request_id }` | `PairCodeUpdate` |
| `device_connection_pair_complete_request` | `{ request_id }` | `()` |
| `device_connection_pair_acknowledge_request` | `{ request_id }` | `()` |
| `device_connection_debug_status` | — | debug counters/state |

### Space sync (websocket data-plane)
| Command | Input | Returns |
|---|---|---|
| `space_sync_list_mappings` | `{ peer_device_id }` | `Vec<space_id>` |
| `space_sync_update_mappings` | `{ peer_device_id, mapped_space_ids }` | mapping state |
| `space_sync_status` | `{ peer_device_id? }` | sync runtime status |

Notes:

- `space_sync` uses durable outbox + ACK replay.
- Data events use a generic envelope with `event_id` + `correlation_id`.
- Hard cut-over naming policy applies (no backward aliases).

General notes:
- Quest ids are UUIDs
- Space ids are strings (`"1"`, `"2"`, `"3"`, or UUID)

## Platform notes

- **Linux**: Sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup to ensure Wayland compatibility
- **Android**: Built via `npm run tauri android build`; project lives in `gen/android/`; convenience signing output is `bin/fini.apk` via `make android-sign-debug`
- **Flatpak**: Packaged via `com.fini.app.yml` at the repo root

## Postponed

- **Voice / ASR** (`src/voice.rs`, `src/model_download.rs`) — offline speech recognition via sherpa-onnx. Code is present but not compiled or registered. Will be revisited after the core quest flow is stable.
