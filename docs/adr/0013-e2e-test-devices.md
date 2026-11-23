# 0013 — E2E tests drive test devices: each a platform, virtual or physical

## Status

Accepted. Written during the design discussion of 2026-10-28. The spike of
D1 chose the Android Emulator on 2026-10-30 (see "Spike results"). Updated
with the project owner on 2026-11-05: the job is the Android e2e job, with
Bluetooth as one of its tests (D3, D5, D8, D10). Updated with the project
owner on 2026-11-06: a test device is a platform with a source (D4), and the
projects `multi-device` and `device-support` (D11).

## Context

The e2e harness in `specs/e2e/actors/fixtures.ts` starts two desktop
`fini-app` processes, or it connects to a TCP port of an app that runs on
another device ("external"). It does not install, start or stop an app on a
phone. The Bluetooth project `actors-ble` uses the `ble-gatt` mock broker
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
the Android job with two phones, local and in CI. The Android tests are in
the project `device-support` (D11): the app start with the notification
channel, and Android-to-Android Bluetooth. The test with three phones (A to B
to C) comes later.

**D4 — The name is `TestDevice`. A test device is a platform, with a
source.** One running Fini app that a test drives is a `TestDevice`. The
fixture is `testDevices`: `testDevices.a`, `testDevices.b`, and
`testDeviceA`, `testDeviceB` by position. A `TestDevice` is not a Fini
*device* (a paired peer, `device_paired_devices`). The words "actor" and
"external" go.

A test device has two separate parts, each a const:

- `TestDevicePlatform`: Linux, Android (later iOS, Meta glasses, ...).
- `TestDeviceSource`: *virtual* (the harness starts it: a `fini-app` process
  on Linux, an emulator on Android) or *physical* (it runs already; the
  harness connects to its TCP port). Each platform can have each source.

A spec names only platforms (`testDevicePlatforms`); their source is virtual.
The physical source comes from `FINI_E2E_TEST_DEVICES`, where each entry
names its platform: `a=linux@tcp:9224,b=android@tcp:9223`. The providers:
`LinuxProvider` and `AndroidProvider` (virtual), `TcpProvider` (physical, each
platform). Appium does the same: `platformName` is one setting, and `avd` or
`udid` another.

**D5 — CI runs the job, but it is not a required check at first.** The job
`android-e2e` (`.github/workflows/android-e2e.yml`) runs `device-support` with
virtual Android test devices on each pull request
that is not a draft. It uploads the logcat of each phone, the emulator logs
and the Playwright results. The project owner makes it a required check after
approximately ten green runs. The release runs the same job as a smoke check
that does not block (`non_blocking: true`): no publish job waits for it.

**D6 — All specs move to `TestDevice`. The old harness is deleted.** The
`LinuxProvider` gets the spawn code of `fixtures.ts`. The `TcpProvider` gets
the "external" code. `specs/e2e/actors/` is deleted. There is no second
harness.

**D7 — Playwright starts the phones, only when a test that runs needs
them.** The entry point is `npm run`. `PhoneGroup` (`phones/phone-group.ts`)
starts the phones on first use. Only the worker-scoped `testDeviceSet`
fixture asks for phones, and only when a test has a virtual Android test
device. When no test that runs needs a phone, no phone starts. The physical
source starts no phone, and a job without `android` in
`FINI_E2E_VIRTUAL_PLATFORMS` skips the test. The first use starts the phones
of the set in one group; their number is the number of virtual Android test
devices in the set. Cuttlefish shared the radio only in one group. The group
writes `android-phones.json` in `RUN_ROOT`, the folder of all run folders. After Playwright restarts a
worker, the group connects to the phones that run. `globalTeardown` stops
the phones only when this file exists.

**D8 — The phones run in one podman image, the same locally and in CI.**
The stage `android-phones-runner` in the `Dockerfile` has Node, Playwright,
adb and the phone tool, with fixed versions. The command
`npm run test:e2e:android` is the same locally and in CI. CI keeps the image
in GHCR, with the hash of the `Dockerfile` as its tag, and builds it only when
the `Dockerfile` changed.

**D9 — Make is for developers on their own computers only. CI calls npm
scripts.** Each workflow step is one `npm run` call. The Make targets call
the same npm scripts. The logic of the Make recipes moves to Node scripts in
`scripts/`. New scripts are TypeScript (`.mts`), which Node 24 runs as they
are.

**D10 — One Android job in CI.** The old job "Android Emulator E2E" (one
emulator, `scripts/android-e2e-assert.sh`) goes. Its check, the app start and
the notification channel `fini.reminders`, is the app-start check of Android
in the project `device-support` (`app-start.spec.ts`, and `appStart` of the
Android entry in `SUPPORTED_DEVICES`).

**D11 — `multi-device` holds the multi-device behaviour; `device-support`
holds the checks of each platform and channel.** Fini will run on many
platforms (ARM boards, glasses, a mirror with a screen), so the e2e tests do
not test each pair, and no project is made for each pair.

- `multi-device`: what Fini does with two or more devices. Approximately 80%
  of its tests run on the pair with the fewest problems: two virtual Linux
  test devices over Network (the default). The other tests are happy paths
  of popular pairs and chains. Each names its test devices and its channel at
  the top of the file, for example `peer-sync-over-ble.spec.ts`:
  `test.use({ testDeviceChannel: ChannelKind.Bluetooth })` (virtual Linux on
  the `ble-gatt` mock radio). The project `multi-device-ble-mock` goes.
- `device-support`: the checks that a platform and its channels work. Each
  spec names its own test devices, any pair or chain. The list
  `SUPPORTED_DEVICES` (`device-support/supported-devices.ts`) only says which
  channels each platform supports; a check skips when a platform of its test
  devices does not support its channel. Matter certification works the same
  way: the device states what it supports (PICS), and only those tests run.
- `testDeviceChannel` (a `ChannelKind`) is a worker option; Playwright starts
  a new worker for a file that changes it. Playwright has its own `channel`
  option (the browser), so the name is `testDeviceChannel`.
- `FINI_E2E_VIRTUAL_PLATFORMS` lists the platforms that a job can start as
  virtual test devices (default `linux`). A test that needs another one skips
  with the reason.
- A CI matrix from `SUPPORTED_DEVICES` comes when a second platform can run in
  CI. Until then the job `android-e2e` runs `device-support`.

## Rules for the Android specs

- Pair through the Add Device screen (`AddDeviceDialog`). Do not call
  `device_save_paired_device`.
- Do not call `space_sync_tick`. The app must sync by itself.
- Prove the channel with `space_sync_session_channel`, not only with the
  green row.
- Keep the Network channel off on the two phones.

## Consequences

- One runner, in TypeScript, for desktop and Android. There is no Python.
- The tests on the `ble-gatt` mock radio stay. It is fast, and it tests the
  protocol.
- Virtual Bluetooth tests the Android Bluetooth API, GATT, pairing,
  permissions and the Fini protocol. It does not test radio chips, radio
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
the host, and it passed all runs. After the spike, the emulator version was
fixed to 36.4.9 (build 14788078): 37.2.13 stops with SIGSEGV on some hosts
(Bazzite). The failed Cuttlefish run was a Fini
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
    `omitDriverInstall` is set. Immediately after the boot of an emulator, this
    stopped the second phone.
  - `adb forward` accepts a connection also when the app does not listen
    yet, so each ping has a time limit.
  - On Bazzite, the emulator stops after approximately 30 s with
    `-gpu swiftshader_indirect`. `angle_indirect` works.
