# E2E on virtual Android phones

The project `android` runs the Fini APK on two virtual Android phones. Its
largest test is Fini Bluetooth between the two Android Bluetooth stacks. The
phones are Android Emulators that share one virtual Bluetooth
medium (netsim). The decisions are in `docs/adr/0013-e2e-test-devices.md`.
The fixture and the other projects are in `docs/e2e.md`.

This lane tests the app start on Android, the notification channel, the
Android Bluetooth API, GATT, pairing, permissions and the Fini protocol. It
does not test radio chips, radio interference, battery use or the address
rotation of each vendor.

| Spec | What it proves |
|---|---|
| `android-app-start.spec.ts` | The app process runs, and the notification channel `fini.reminders` exists. |
| `bluetooth-pair-from-zero.spec.ts` | A new pair over Bluetooth through Add Device, until the Bluetooth row is green on both phones. |

A spec runs shell commands on its phone with `testDevice.shell(command)`. Only
an `android-phone` test device has it.

## Parts

```
npm run test:e2e:android [-- <playwright args>]
└── scripts/e2e-android.mts
    ├── on the host: podman pull of the GHCR image (CI), or podman build --target android-phones-runner
    │               podman run --device /dev/kvm --shm-size=2g -v <repo>:/work:Z
    └── in the container (FINI_E2E_CONTAINER_RUNNER=1): npx playwright test --project=android
        └── fixture testDevices: a=android-phone, b=android-phone
            ├── PhoneGroup (phones/phone-group.ts)
            │   ├── android-phones.json    the serials, in RUN_ROOT
            │   └── AndroidEmulator (phones/android-emulator.ts), a PhoneTool
            │       └── emulator-5554, emulator-5556 on one netsimd
            ├── AndroidPhoneProvider (providers/android-phone.ts), one for each phone
            │   └── devtools bridge: app TCP 9223 → adb forward → host port 9223, 9225
            └── global-teardown.ts → PhoneGroup.stopRunning()
```

## The image

The `Dockerfile` stage `android-phones-runner` (Debian trixie) has Node 24,
the Android command-line tools, platform-tools (adb), the system image
`android-34;google_apis;x86_64`, and one AVD `fini-e2e` (`pixel_6`). The
emulator version is fixed: build 14788078 (36.4.9). Emulator 37.2.13 stops
with SIGSEGV on some hosts (Bazzite).

The container needs only `/dev/kvm`. It has its own network, so its adb server
does not touch the adb server of the host. The repo, with `node_modules`,
comes in through the mount.

## When the phones start

Phones start only when a test that runs has an `android-phone` test device
(ADR-0013 D7). `--project ui`, `--project multi-device`, and runs with
physical devices only start no phone.

1. The first request calls `PhoneGroup.shared().serials(n)`. `n` is the
   project option `phoneCount` (2).
2. `AndroidEmulator.up(n)` starts each emulator with `-read-only`, 5 s after
   the one before it. Two starts in the same second on one read-only AVD can
   fail.
3. It waits until `sys.boot_completed` is 1 and `pm path android` answers
   (360 s limit). Else it throws `PhoneBootError`, with the prefix
   `[infrastructure]`.
4. `PhoneGroup` writes the serials to `android-phones.json`. When Playwright
   starts a worker again, the group reads this file, checks
   `adb get-state`, and uses the same phones.
5. `globalTeardown` stops the phones only when the file exists. With
   `FINI_E2E_KEEP_PHONES=1` the phones stay on for the next run.

## How the app starts on a phone

`AndroidPhoneProvider.prepare()`, for each test file:

1. `_android.devices({ omitDriverInstall: true })` finds the phone by serial.
   The driver app of Playwright stopped a phone that had just booted.
2. `adb install -r -g` installs the APK (180 s limit, 3 tries).
   `installApk` sends the 455 MB debug APK in one write, which fails with
   EPIPE.
3. `pm clear`, then `pm grant` for `BLUETOOTH_SCAN`, `BLUETOOTH_CONNECT`,
   `BLUETOOTH_ADVERTISE` and `POST_NOTIFICATIONS`.
4. `cmd bluetooth_manager enable`.
5. `svc power stayon true`, `locksettings set-disabled true`,
   `wm dismiss-keyguard`. A webview behind the lock screen or in the
   background slows its timers, and each `eval` then times out.
6. `am start -W`, then `adb forward tcp:<host port> tcp:9223`.

`connect()` tries the bridge for 180 s, with a 5 s limit on each ping, because
`adb forward` accepts a connection also when the app does not listen yet. Each
30 s, it starts the app again if its process is not there. `dispose()` saves
the logcat and removes the port forward. The phone stays on.

## Rules for the specs

- Pair through the Add Device screen (`AddDeviceDialog`). Do not call
  `device_save_paired_device`.
- Do not call `space_sync_tick`. The app must sync by itself.
- Prove the channel with `space_sync_session_channel`, not only with the
  green row.
- Keep the Network channel off on the two phones.

## Build and run

1. Build the x86_64 debug APK with devtools: `npm run build:android:e2e`
   (local shortcut: `make android-build-emulator-e2e`). Another APK:
   `FINI_E2E_ANDROID_APK=<path>`.
2. Run the lane: `npm run test:e2e:android`.

On physical devices, `FINI_E2E_TEST_DEVICES=a=tcp:9224,b=tcp:9223` runs
Playwright on the host, with no container and no emulator.

CI: the job `android-e2e` in `.github/workflows/android-e2e.yml` builds
the APK, enables KVM and runs the same command. It uploads
`tmp/fini-e2e-multi-device/` and `specs/e2e/test-results/`, and shows the free
disk space at the end. It is not a required check yet (ADR-0013 D5).
The image is kept in GHCR as
`ghcr.io/<owner>/fini-android-phones-runner-cache:<sha256 of the Dockerfile>`
(`FINI_ANDROID_PHONES_CACHE_PREFIX`). The script pulls it. When the
Dockerfile changed, it builds the image and pushes it
(`FINI_ANDROID_PHONES_CACHE_PUSH=1`; not for pull requests from forks).
Local runs set no prefix and use the layer cache of podman.
`ci.yml` calls it for each pull request. `release-tag.yml` calls it with
`non_blocking: true`: no publish job waits for it, and a failure does not fail
the release.

## Logs

| What | Where |
|---|---|
| emulator output | `RUN_ROOT/emulator-<port>.log` |
| logcat of each phone | `<run folder>/test-devices/<slug>.logcat.txt` |
| step times | the test annotations in the Playwright report |

`RUN_ROOT` is `tmp/fini-e2e-multi-device/` by default.

Environment variables: `FINI_E2E_AVD` (default `fini-e2e`),
`FINI_E2E_EMULATOR_GPU` (default `angle_indirect`; `swiftshader_indirect`
stops the emulator after about 30 s on Bazzite), `FINI_E2E_PACKAGE`
(default `com.fini.app.debug`).

## Known faults

- Sometimes the first app start after a fresh boot does not answer on the
  bridge ("no answer on tcp:9225"). The cause is not known. The restart check
  in `connect()` catches most cases.
- On a fresh boot, Google Play services can stop answering (ANR). Android
  then stops it, and also stops Fini, because the WebView uses the font
  provider of Play services. Android starts Fini again for
  `SyncForegroundService` only, with no window, so the bridge has no page.
  The next test fails with "This socket has been ended by the other party".
  `pidof` still finds the process. Logcat shows "Killing ... com.fini.app.debug
  ... depends on provider com.google.android.gms/.fonts.provider.FontsProvider".
  Seen in CI run 41966860298. This is possibly also the cause of the fault
  above.

## Not done yet

- `bluetooth-channel-add-again` and `peer-sync-over-ble` on the phones (they
  need a "pair if needed" helper).
- Recovery from an app that Android stopped between tests: start
  `MainActivity` again and connect before each test (as Detox
  `device.launchApp` does).
