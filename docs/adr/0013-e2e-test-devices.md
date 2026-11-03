# 0013 — E2E tests drive test devices: desktop processes, virtual phones, physical devices

## Status

Accepted. Written during the design discussion of 2026-10-28. The spike of
D1 chose the Android Emulator on 2026-10-30 (see "Spike results").

## Context

The e2e harness in `specs/e2e/actors/fixtures.ts` starts two desktop
`fini-app` processes, or it connects to a TCP port of an app that runs on
another device ("external"). It does not install, start or stop an app on a
phone. The Bluetooth lane `actors-ble` uses the `ble-gatt` mock broker
between two desktop processes. The Android CI job starts one emulator and
checks only that the app starts.

No test runs Fini Bluetooth between two Android Bluetooth stacks. A test
with three phones (A to B to C) is not possible.

There are no physical Bluetooth devices for CI. The tests must run in the
cloud on virtual phones.

Designs compared:

- Google Mobly: a Python runner for tests on many Android devices. It
  connects to devices; it does not start them.
- AOSP Cuttlefish: a virtual Android device on KVM.
  `cvd create --num_instances=N` starts N phones that share one virtual
  Bluetooth medium (RootCanal).
- Android Emulator with netsim: the emulator that CI uses now. netsim
  connects the virtual Bluetooth of all emulators on one host.
- OmniLab ATS: a lab system that gives devices to tests. It is too large for
  one test with two or three phones.
- Playwright `_android`: an experimental Playwright API. It connects to
  Android devices by adb serial.
- Appium with WebdriverIO: native Android UI. It is not necessary while the
  Tauri bridge gives all UI actions.

## Decisions

**D1 — A spike compares Cuttlefish and the Android Emulator. Fini keeps one
of them.** The spike starts two phones with each tool, installs the Fini
debug APK, and runs `bluetooth-pair-from-zero` on them. Fini keeps the tool
that passes and is faster and more stable over approximately five runs. Fini
keeps only one tool, also when the two tools pass. The other tool stays in
git history.

**D2 — The tests drive the app through the Tauri bridge. Device work goes
through Playwright `_android`.** The app actions use `PluginClient` over TCP
port 9223 after `adb forward`, as on a physical phone now. `_android` finds
the phones by serial and runs `pm grant` through `shell`, with
`omitDriverInstall: true`. Two jobs use the adb command:

- The install. `installApk` sends the 455 MB debug APK in one write, and
  the write fails with EPIPE.
- The port forward. `_android` has no port forward.

**D3 — Scope of the first pull request.** The spike, the new fixture, and
a lane with two phones for Android-to-Android Bluetooth, local and in CI.
The test with three phones (A to B to C) comes later.

**D4 — The name is `TestDevice`.** One running Fini app that a test drives
is a `TestDevice`. The fixture is `devices`: `devices.a`, `devices.b`. A
`TestDevice` is not a Fini *device* (a paired peer, `device_paired_devices`).
The words "actor" and "external" go. A device that a person connects by USB
or Wi-Fi is a *physical device*.

**D5 — CI runs the lane, but it is not a required check at first.** The job
`android-ble-e2e` runs on each pull request that is not a draft. It uploads
logcat, the Bluetooth packet trace and the Playwright trace. The project
owner makes it a required check after approximately ten green runs.

**D6 — All specs move to `TestDevice`. The old harness is deleted.** The
desktop-process provider gets the spawn code of `fixtures.ts`. The
physical-device provider gets the "external" code. `specs/e2e/actors/` is
deleted. There is no second harness.

**D7 — Playwright starts the phones, only when a test that runs needs
them.** The entry point is `npm run`. The `phones` fixture is worker-scoped
and lazy. Only the `devices` fixture uses it, and only when a test has a
device of the kind `android-phone`. When no test that runs needs a phone, no
phone starts. Physical devices in `FINI_E2E_DEVICES` start no phone. The
first use starts all phones of the project in one group, because
Cuttlefish shares the radio only in one group. The fixture writes
`android-phones.json` in the run folder. After Playwright restarts a worker,
the fixture connects to the phones that run. `globalTeardown` stops the
phones only when this file exists.

**D8 — The phones run in one podman image, the same locally and in CI.**
The stage `android-phones-runner` in the `Dockerfile` has Node, Playwright,
adb and the phone tool, with fixed versions. The command
`npm run test:e2e:android-ble` is the same locally and in CI.

**D9 — Make is for developers on their own computers only. CI calls npm
scripts.** Each workflow step is one `npm run` call. The Make targets call
the same npm scripts. The logic of the Make recipes moves to Node scripts in
`scripts/`.

## Rules for the Android Bluetooth lane

- Pair through the Add Device screen (`AddDeviceDialog`). Do not call
  `device_save_paired_device`.
- Do not call `space_sync_tick`. The app must sync by itself.
- Prove the channel with `space_sync_session_channel`, not only with the
  green row.
- Keep the Network channel off on the two phones.

## Consequences

- One runner, in TypeScript, for desktop and Android. There is no Python.
- The `ble-gatt` mock broker lane stays. It is fast, and it tests the
  protocol.
- Virtual Bluetooth tests the Android Bluetooth API, GATT, pairing,
  permissions and the Fini protocol. It does not test real radio chips, radio
  interference, battery use or the address rotation of each vendor.

## Spike results

The spike ran `bluetooth-pair-from-zero` on two phones with each tool, in
the `android-phones-runner` podman image, on 2026-10-28 to 2026-10-30. The
results are from the last version of the harness, from a fresh boot of the
phones.

| | Android Emulator | Cuttlefish |
|---|---|---|
| Version | emulator 37.2, API 34 `google_apis` x86_64 | AOSP build 16644040, Android 17 |
| Container | `/dev/kvm` only | `--privileged`, vsock, vhost-net, tap devices |
| Boot of two phones | 33 s | less than 1 min |
| Fresh boot and 5 runs | 170 s and 191 s | 353 s and 406 s |
| Passed | 10 of 10 | 9 of 10 |
| Listing time | 0.0 to 0.6 s | 0.0 to 0.4 s |

Fini keeps the Android Emulator (D1): it is faster, it needs less access to
the host, and it passed all runs. The failed Cuttlefish run was a Fini
defect (issue #198), not a defect of the tool.

What the spike found:

- Fini defects, now in issue #198 and in the fixes of PR #197:
  - Android rejects a scan start after five starts in 30 s, with no error to
    the app. Fini then waited a full scan time (20 s). PR #197 counts the
    starts. In one app process, the wait for a free start can still be
    longer than the 10 s standard (11.0 s).
  - A JNI abort when the Bluetooth driver started after the activity was
    destroyed. PR #197 fixed it (ble-gatt 4adef4f).
  - Once, a session from the peer arrived before the pair was stored, and
    was rejected ("pinned for no pair"). The channel stayed grey.
- Harness facts:
  - A webview behind the lock screen or in the background slows its timers,
    and each `eval` times out. The provider keeps the screen on and
    unlocked, and the app in front.
  - The ping proves only the Rust side of the bridge. The provider waits
    for the page.
  - `_android.devices()` installs a driver app on each phone unless
    `omitDriverInstall` is set. On an emulator that had just booted, this
    stopped the second phone.
  - `adb forward` accepts a connection also when the app does not listen
    yet, so each ping has a time limit.
  - On Bazzite, the emulator stops after approximately 30 s with
    `-gpu swiftshader_indirect`. `angle_indirect` works.
