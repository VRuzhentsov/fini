---
name: android-testing
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

## Validated findings (2026-04-03)

1. Android app automation works in the normal development flow.
2. Debug/dev runtime exposes session tooling.
3. Session routing benefits from explicit transport mapping in multi-target environments.
4. Identity verification via backend state is the authoritative proof of Android target.
5. Webview automation tools (`webview_interact`, `webview_wait_for`, `webview_execute_js`) drive Android navigation successfully.

## Device automation workflow

### 1. Start Android runtime

- Run `make android-dev` and keep it active.
- Use `make android-devices` and `make android-connect` when device discovery/connectivity setup is needed.

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
