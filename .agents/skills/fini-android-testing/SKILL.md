---
name: fini-android-testing
description: Validate the Fini Android app on a real device or emulator through a device-automation workflow. Use this when users ask to verify Android behavior, prove navigation and state changes, debug mobile-only issues, or execute app automation against `com.fini.app`.
---

# Android Testing

Use this skill to test Android through the available device automation tools as the primary interaction layer.

## Goal

Prove Android behavior with a complete evidence chain:

1. session target identity confirms Android runtime
2. automation actions demonstrate control
3. screenshots and state snapshots confirm visible result

## Repo specifics

- Package: `com.fini.app`
- Preferred runtime commands:
  - `make android-devices`
  - `make android-connect`
  - `make android-dev`
- App automation in this repo runs in debug/dev runtime, so Android validation uses the dev flow.
- `make android-connect` is bounded by `ADB_CONNECT_TIMEOUT` and must not be retried in a loop when it times out. If mDNS finds a device but `make android-devices` remains empty after one connect attempt, report the connection as blocked and ask the user to re-authorize wireless debugging or use USB/emulator.

## Validated findings (2026-04-03)

1. Android app automation works in the normal development flow.
2. Debug/dev runtime exposes session tooling.
3. Session routing benefits from explicit transport mapping in multi-target environments.
4. Identity verification via backend state is the authoritative proof of Android target.
5. Webview automation tools (`webview_interact`, `webview_wait_for`, `webview_execute_js`) drive Android navigation successfully.

## Android debug logging (2026-06-26)

Tauri's Kotlin `Logger` gates ALL log levels — verbose, debug, info, warn, **error** — behind `BuildConfig.DEBUG`. When `BuildConfig.DEBUG = false` (release profile), every `Logger.error()` call in a plugin's catch block is silently dropped. This means plugin exceptions, `invoke.reject()` messages, and command routing errors are invisible in the default `make android-debug-deploy` output.

**Rule**: when a Tauri plugin command returns `null` or fails silently on Android with no JS-visible error, always reproduce with `make android-debug-deploy-debug` first to enable Tauri Kotlin logs before investigating the Rust or JS side.

- `make android-debug-deploy` → release profile APK → `BuildConfig.DEBUG=false` → Tauri logs OFF
- `make android-debug-deploy-debug` → debug profile APK → `BuildConfig.DEBUG=true` → Tauri logs ON

Logcat filter to isolate Tauri plugin traffic:
```
adb -s <ANDROID_SERIAL> logcat -s "Tauri" -s "Tauri/Plugin"
```

## Tauri Android runtime quirk (2026-06-26)

`PluginManager` (Kotlin `object` singleton in `app.tauri.plugin`) has two `lateinit` fields — `activity` and `startActivityForResultLauncher` — that are initialized in `PluginManager.onActivityCreate(activity)`. Tauri's Rust bootstrap (`tauri-2.11.0`) never calls this method via JNI; it only calls `onWebViewCreated`, `load`, and `runCommand`.

Consequence: any plugin command that calls `PluginManager.startActivityForResult` (e.g. file picker, permission requests) throws `UninitializedPropertyAccessException` at runtime, which is caught, logged (only when DEBUG=true), and forwarded as `invoke.reject()` → JS receives `null`.

**Fix**: call `PluginManager.onActivityCreate(this)` in `MainActivity.onCreate` **before** `super.onCreate()`. `registerForActivityResult` must be called before `onStart`, so calling it in `onCreate` is correct and safe.

## Device automation workflow

### 1. Start Android runtime

- Run `make android-dev` and keep it active.
- Run `make android-devices` before attempting Wi-Fi connect.
- Use `make android-connect` once when device discovery/connectivity setup is needed; if it times out or devices remain empty, stop instead of repeatedly running ADB connect.

### 2. Establish automation session

- Start session with `driver_session` (`action: start`) using explicit host/port.
- In multi-target environments, use explicit transport mapping and a dedicated host port for deterministic routing.

### 3. Prove target identity before scenario execution

Collect target proof at session start:

- `driver_session` (`action: status`)
- `ipc_get_backend_state`

Validate these fields:

- `environment.os == "android"`
- `environment.arch == "aarch64"`
- `app.identifier == "com.fini.app"`

Optional corroboration:

- `webview_execute_js` returns Android WebView user agent and active route.

### 4. Execute scenario through automation tools

Use webview and app automation tools as the scenario control surface:

- `webview_dom_snapshot` to locate interactive refs
- `webview_interact` to click/focus/scroll
- `webview_wait_for` to confirm transition checkpoints
- `webview_execute_js` to assert route/state and produce deterministic probes
- `webview_screenshot` to capture visual evidence

### 5. Capture evidence artifacts

For each meaningful transition, capture:

- pre-action screenshot
- post-action screenshot
- route/state payload from `webview_execute_js` when route/state is part of the claim
- DOM snapshot excerpt when structure/labels are part of the claim

### 6. Session re-validation checkpoints

Re-run identity proof after reconnects, port changes, or route reconfiguration:

- `driver_session status`
- `ipc_get_backend_state`

Classify evidence according to current verified target identity.

## Evidence standard

For every claim, present:

1. **Target proof** - session status + backend state identity
2. **Action proof** - exact automation calls used
3. **Result proof** - visible state change and route/state payload
4. **Artifacts** - screenshot paths and relevant snapshot lines

## Output format

### Result

- One sentence on what was proven on Android through device automation.

### Session target proof

- `driver_session status` result
- `ipc_get_backend_state` result
- optional user-agent and route probe result

### Automation actions

- ordered list of automation interactions
- checkpoints used to validate each transition

### Evidence

- before/after screenshot paths
- key DOM snapshot lines or route payloads
- visible and structural state changes

### Limits

- remaining unknowns or items outside current proof scope

## Example scenario: Settings -> Device page

1. Capture initial snapshot.
2. Click `Settings` nav link via `webview_interact`.
3. Wait for `Settings` via `webview_wait_for`.
4. Capture screenshot.
5. Click device row link from settings list.
6. Wait for `MAPPED SPACES`.
7. Capture screenshot and route payload confirming `#/settings/device/...`.
8. Report target proof, actions, and artifacts.
