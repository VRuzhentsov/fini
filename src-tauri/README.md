# src-tauri/

Rust backend of the Fini app, powered by Tauri 2.0.

## Structure

```
src-tauri/
├── src/
│   ├── lib.rs         # Shared app library — DB setup, models, command handlers
│   ├── schema.rs      # Diesel table definitions
│   ├── desktop.rs     # Desktop GUI binary entry point (`fini-app`)
│   └── cli.rs         # CLI-only binary entry point (`fini`)
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

See `specs/` at the repo root for domain model specs ([[Quest]], [[Space]], [[RepeatRule]], [[QuestSeries]], [[QuestOccurrence]], [[Reminder]], [[FocusHistory]], [[DeviceConnection]], [[SpaceSync]], [[Network]]).

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

- **Linux GUI**: Applies default WebKit startup guards before Tauri initializes:
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` for Wayland/mesa stability,
  `WEBKIT_DISABLE_SANDBOX=1` for SELinux-enforcing AppImage hosts,
  `WEBKIT_DISABLE_COMPOSITING_MODE=1` to force software compositing when the
  bundled WebKitGTK's accelerated compositing path is unstable against the
  host's mesa/DRM/GL stack, and `LIBGL_ALWAYS_SOFTWARE=1` as a stronger
  Mesa-side fallback for repeated AppImage WebKit aborts on host GL drivers.
  Existing environment values are preserved for explicit local diagnostics.
  The resolved value of each guard is logged to stderr at startup
  (`[webkit-runtime] KEY=value`) so a packaged AppImage session can confirm
  which flags actually reached `WebKitWebProcess`.
- **Desktop GUI**: `fini-app` is the bundled GUI binary and is built with `ui-plane,desktop-updater`.
  Release builds embed the Tauri updater public key with `FINI_TAURI_UPDATER_PUBKEY`,
  publish the signed desktop updater manifest at
  `https://github.com/VRuzhentsov/fini/releases/latest/download/latest.json`, and
  check it automatically at startup when Settings -> Updates -> Automatic updates
  is enabled. Turning that setting off skips the next startup auto-update check;
  set `FINI_DISABLE_AUTO_UPDATE=1` to skip the startup check for diagnostics, or
  `FINI_DESKTOP_UPDATE_ENDPOINT` / `FINI_DESKTOP_UPDATE_PUBKEY` for staging channels.
- **CLI/runtime**: `fini` is the CLI-only binary and is built with `cli-plane`
- **Android**: Built via `npm run tauri android build`; project lives in `gen/android/`
  - Android builds must pass `--features ui-plane` only so CLI modules and dependencies are excluded from the mobile bundle
  - `make android-debug-deploy` builds, signs, installs, and launches a local debug-keystore APK using git-derived `versionName` and `versionCode`
  - `make android-release-deploy-local` performs the same local build/install flow but signs with release-lineage credentials from `ANDROID_KEYSTORE_PATH` or `ANDROID_KEYSTORE_BASE64` plus the matching password and alias env vars
  - local debug output is `bin/fini.apk`; local release-signed output is `bin/fini-release.apk`
- **Flatpak**: Packaged via `com.fini.app.yml` at the repo root

## Local AppImage build failures on newer Linux toolchains

`make build` (and `make flatpak-install-local`) default `NO_STRIP=true` (set in the
root `Makefile`). Without it, local AppImage bundling on newer host toolchains
(e.g. Fedora 44+, glibc/binutils new enough to emit `.relr.dyn` relocation
sections) fails with:

```text
ERROR: Strip call failed: .../usr/bin/strip: <library>: unknown type [0x13] section `.relr.dyn'
```

`linuxdeploy` vendors its own `strip` binary, which predates RELR section
support, and aborts bundling before producing an AppImage — the host's own
(newer) `strip` is not used regardless of `PATH` or a `STRIP` env var, since
linuxdeploy's bundled AppImage mounts its own `usr/bin/strip`. `NO_STRIP=true`
skips the strip step entirely, at the cost of larger, unstripped local
binaries. Override with `NO_STRIP=false` if your toolchain doesn't hit this.
CI release builds (`.github/workflows/release-tag.yml`,
`release-dry-run.yml`) call `npm run tauri build` directly on
`ubuntu-latest`, not through this Makefile target, so this default has no
effect on published release artifacts.

## Linux AppImage WebKit crash reports

Fini starts Linux WebKit with default guards from `src-tauri/src/webkit_runtime.rs`: `WEBKIT_DISABLE_DMABUF_RENDERER=1`, `WEBKIT_DISABLE_SANDBOX=1`, `WEBKIT_DISABLE_COMPOSITING_MODE=1`, and `LIBGL_ALWAYS_SOFTWARE=1`.
Each guard's resolved value is logged to stderr at startup (`[webkit-runtime] KEY=value`) — capture those lines alongside any crash report to confirm the flags reached the AppImage's `WebKitWebProcess`.
If `WebKitWebProcess` aborts in an AppImage build, capture a sanitized report that keeps the failure actionable without exposing host identifiers.

Report only:

- Fini version and install channel (`AppImage`)
- Linux distro and session type (`Wayland` or `X11`)
- The last user action before the abort
- Whether the crash reproduces in a fresh Fini profile
- The `[webkit-runtime]` startup log lines showing the resolved guard values
- A redacted `coredumpctl info` or journal excerpt that keeps the process name, signal, package/build, and stack summary

Do not publish raw coredumps or any user, host, machine, boot, session, or transient mount identifiers.
Do not include local core storage paths, AppImage mount paths, or usernames in public reports.

For public GitHub reports, use `.github/ISSUE_TEMPLATE/linux-appimage-webkit-crash.yml`; it requires the same sanitized fields and includes a privacy checklist before submission.

Suggested local notes template:

```text
Fini version:
Install channel: AppImage
Linux distro:
Session type:
Trigger action:
Fresh profile repro:
webkit-runtime startup log lines:
Sanitized coredump summary:
Stack summary:
Rendering workaround comparison:
```

## Postponed

- **Voice / ASR** (`src/voice.rs`, `src/model_download.rs`) — offline speech recognition via sherpa-onnx. Code is present but not compiled or registered. Will be revisited after the core quest flow is stable.
