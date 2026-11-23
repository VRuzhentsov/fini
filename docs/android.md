# Android development

Facts for work with the Fini Android app on a physical phone or an emulator:
the packages, the logs, known Tauri faults, and the devtools bridge. Virtual
Android test devices in e2e are in [`e2e-android.md`](e2e-android.md). The
build and deploy commands are in [`development.md`](development.md),
"Commands", and `README.md`, "Build (Android)".

## Packages and runtime

- Package: `com.fini.app` (Play Store release). `make android-debug-deploy` installs the true `debug` buildType as a separate `com.fini.app.debug` package instead (see `src-tauri/gen/android/app/build.gradle.kts`'s `applicationIdSuffix`) -- coexists safely alongside a Play-Store-installed `com.fini.app` on the same device, no certificate conflict, no uninstall ever required. Confirm which one an automation session is actually attached to via `ipc_get_backend_state`'s `app.identifier` before trusting device state.
- The `mcp___hypothesi_tauri-mcp-server__*` driver_session/webview_*/ipc_* tools can attach to the `com.fini.app.debug` build (it's `devtools`-capable, unlike the Play Store release). Reach for that connection only when genuine IPC-level introspection is actually needed -- prefer plain `adb`/logcat/screenshots for observation and the devtools bridge for interaction otherwise; the tauri-mcp bridge is resource-heavy and should not be the default interaction path.
- Preferred runtime commands: `make android-devices`, `make android-connect`, `make android-dev`.
- App automation in this repo runs in debug/dev runtime, so Android validation uses the dev flow.
- `make android-connect` is bounded by `ADB_CONNECT_TIMEOUT` and must not be retried in a loop when it times out. If mDNS finds a device but `make android-devices` remains empty after one connect attempt, the connection is blocked: the user re-authorizes wireless debugging or uses USB/emulator.

## Validated findings (2026-04-03)

1. Android app automation works in the normal development flow.
2. Debug/dev runtime exposes session tooling.
3. Session routing benefits from explicit transport mapping in multi-target environments.
4. Identity verification via backend state is the authoritative proof of Android target.
5. Webview automation tools (`webview_interact`, `webview_wait_for`, `webview_execute_js`) drive Android navigation successfully.

## Android debug logging (2026-06-26)

Tauri's Kotlin `Logger` gates ALL log levels — verbose, debug, info, warn, **error** — behind `BuildConfig.DEBUG`. When `BuildConfig.DEBUG = false` (release profile), every `Logger.error()` call in a plugin's catch block is silently dropped. This means plugin exceptions, `invoke.reject()` messages, and command routing errors are invisible in the default `make android-release-deploy-debugsigned` output.

**Rule**: when a Tauri plugin command returns `null` or fails silently on Android with no JS-visible error, always reproduce with `make android-debug-deploy` first to enable Tauri Kotlin logs before investigating the Rust or JS side.

- `make android-release-deploy-debugsigned` → release buildType APK, debug-keystore signed, shares `com.fini.app`'s package id → `BuildConfig.DEBUG=false` → Tauri logs OFF
- `make android-debug-deploy` → true debug buildType APK, separate `com.fini.app.debug` package id → `BuildConfig.DEBUG=true` → Tauri logs ON

Logcat filter to isolate Tauri plugin traffic:
```
adb -s <ANDROID_SERIAL> logcat -s "Tauri" -s "Tauri/Plugin"
```

## Tauri Android runtime quirk (2026-06-26)

`PluginManager` (Kotlin `object` singleton in `app.tauri.plugin`) has two `lateinit` fields — `activity` and `startActivityForResultLauncher` — that are initialized in `PluginManager.onActivityCreate(activity)`. Tauri's Rust bootstrap (`tauri-2.11.0`) never calls this method via JNI; it only calls `onWebViewCreated`, `load`, and `runCommand`.

Consequence: any plugin command that calls `PluginManager.startActivityForResult` (e.g. file picker, permission requests) throws `UninitializedPropertyAccessException` at runtime, which is caught, logged (only when DEBUG=true), and forwarded as `invoke.reject()` → JS receives `null`.

**Fix**: call `PluginManager.onActivityCreate(this)` in `MainActivity.onCreate` **before** `super.onCreate()`. `registerForActivityResult` must be called before `onStart`, so calling it in `onCreate` is correct and safe.

## The devtools bridge

`.agents/skills/fini-android-testing/devtools-bridge.mjs` is a thin bridge to
a running debug app's devtools socket: it connects, evaluates the expression
you give it **inside the app's webview**, prints the result, and exits.
Seconds, not minutes — no test devices to start, no test runner, no build.

```bash
# from the repo root, so it resolves the repo's node_modules
node .agents/skills/fini-android-testing/devtools-bridge.mjs 9224 "<javascript>"
```

Ports: **9224** desktop (`make desktop-debug`, or `make desktop-debug-dev`
for the same app with the frontend over HMR), **9223** phone (after
`adb forward tcp:9223 tcp:9223`).

The DOM is in scope, and `invoke(cmd, args)` is provided as a shorthand for
the Tauri bridge. The last expression is the result; wrap in an async IIFE
when you need `await`.

```bash
node ... 9224 "invoke('device_paired_devices')"
node ... 9223 "[...document.querySelectorAll('button')].map(b => b.textContent.trim())"
node ... 9224 "[...document.querySelectorAll('button')].find(b => b.textContent.includes('Pair')).click()"
node ... 9224 "(async () => { const p = await invoke('device_paired_devices');
   for (const d of p) await invoke('device_unpair', { peerDeviceId: d.peer_device_id });
   return p.length; })()"
```

The bridge also works on a virtual Android test device: the `device-support`
run forwards phone A to host port 9223 and phone B to 9225 while a test runs
(inside the container). See [`e2e-android.md`](e2e-android.md).

**It deliberately has no verbs.** Compose the expression for the task at
hand rather than adding a named action to the script — the bridge knows
nothing about Fini, and all the knowledge lives in the command. Adding
`unpair-all`-style helpers back is the thing to avoid.

**Use it instead of `adb shell input tap`.** A selector addresses an element
inside the webview, so it cannot land in another app and it fails loudly when
the element is absent. A coordinate silently pokes whatever is underneath —
which, on a phone that is someone's daily device, has meant tapping into an
unrelated app. Reserve `adb shell input` for native Android UI the webview
cannot reach (permission dialogs), and screenshot immediately before each tap.

**Use it instead of writing a spec** when the goal is to *do* something once:
unblock a flow, read state, click through a screen. Specs are for coverage
the repo keeps; this is for hands.

Worth knowing: it was built after several hours were lost running the full
e2e harness to perform single actions, each round trip costing minutes with
no visible progress. The whole pairing flow — unpair both sides, open Add
Device, Pair, Accept, type the confirmation code, enable Bluetooth — was
then driven through it in about five minutes.

## Virtual Android test devices

For Bluetooth between two Android stacks without hardware, use the
`device-support` specs on two virtual Android test devices (ADR-0013), not a
physical phone:

```bash
npm run build:android:e2e          # x86_64 debug APK with devtools (make android-build-emulator-e2e)
npm run test:e2e:android           # android e2e image, two emulators on one netsim radio
```

[`e2e-android.md`](e2e-android.md) has the details.
