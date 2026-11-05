# E2E tests

The e2e tests drive the real Fini app: the CLI, one desktop window, or two or
more apps that pair and sync. Playwright Test is the one runner. The decisions
are in `docs/adr/0013-e2e-test-devices.md`. The virtual Android phones are in
`docs/e2e-android.md`. What each lane proves for each transport is in
`specs/e2e/transports.md`.

## Parts

```
specs/e2e/
├── playwright.config.ts          projects; globalTeardown
├── fixtures/cli.ts               createCliClient(): the fini binary with a temp database
├── ui/fixtures.ts                tauriPage: one desktop app for each test
├── components/                   one class for each part of the screen (AddDeviceDialog, DevicePage, ...)
└── multi-device/
    ├── test-device.ts            TestDevice; fixtures testDevices, testDeviceA, testDeviceB
    ├── test-device-set.ts        TestDeviceSet: one provider for each test device
    ├── run-root.ts               RUN_ROOT: the folder of all run folders
    ├── providers/
    │   ├── desktop-process.ts    DesktopProcessProvider: starts fini-app, unix socket bridge
    │   ├── android-phone.ts      AndroidPhoneProvider: the app on a virtual phone
    │   ├── physical-device.ts    PhysicalDeviceProvider: an app that runs already, on a TCP port
    │   └── ble-mock-broker.ts    BleMockBroker: the mock radio of multi-device-ble-mock
    ├── phones/                   virtual phones (docs/e2e-android.md)
    ├── global-teardown.ts        stops the virtual phones, if a test started them
    ├── helpers/                  pairing and sync set-up (device-sync, personal-sync, ble-sync, ...)
    └── tests/                    the multi-device specs
```

## TestDevice

A `TestDevice` is one running Fini app that a test drives (glossary). It is
not the Fini `Device` model. A `TestDevice` has:

- `page`: a `TauriPage` for the screen.
- `invoke<T>(command, args)`: a call to a Tauri command.
- `kind`: `desktop-process`, `android-phone` or `physical-device`.

A test gets its test devices from the fixture `testDevices` (by slug), or from
`testDeviceA` and `testDeviceB` (by position). The components in
`components/` take a `TauriPage`. They know nothing about test devices.

## How the test devices start

```
fixture testDevices (worker-scoped)
└── TestDeviceSet.fromEnv(FINI_E2E_TEST_DEVICES ?? testDeviceKinds, phoneCount)
    ├── desktop-process  → DesktopProcessProvider  (and BleMockBroker when FINI_E2E_TRANSPORT=ble)
    ├── android-phone    → PhoneGroup.shared().serials(n) → AndroidPhoneProvider
    └── tcp:<port>       → PhysicalDeviceProvider
        each provider: prepare() → connect() → TestDevice waits for the page
```

- `FINI_E2E_TEST_DEVICES` is a list `slug=kind,...`, for example
  `a=desktop-process,b=tcp:9223`. Without it, the project option
  `testDeviceKinds` applies. The default is `a=desktop-process,b=desktop-process`.
- A set has two test devices or more.
- `dispose()` releases only what the provider made. A physical device stays
  as it is.
- Each run has a folder in `RUN_ROOT` (default `tmp/fini-e2e-multi-device/`).
  The folder holds the app data and the logs. The set deletes it after a
  passed run. With `FINI_E2E_KEEP=1`, or after a failed run, the set keeps it.

## Projects

| Project | Test devices | Specs |
|---|---|---|
| `cli` | none: the `fini` binary | `reminder-bridge.spec.ts`, `feature-plane-cli.spec.ts` |
| `ui` | one desktop app for each test | `ui/tests/` |
| `multi-device` | two desktop processes, Network channel | `multi-device/tests/`, without `peer-sync-over-ble` |
| `multi-device-ble-mock` | two desktop processes on the `ble-gatt` mock radio (`FINI_E2E_TRANSPORT=ble`) | `peer-sync-over-ble.spec.ts` |
| `android` | two virtual Android phones (`docs/e2e-android.md`) | `android-app-start.spec.ts`, `bluetooth-pair-from-zero.spec.ts` |

Playwright runs one worker, and the files in name order. A file that needs
the state of earlier files starts with `zz-`.

## Commands

CI calls only npm scripts. The Make targets are shortcuts for developers to
the same scripts (ADR-0013 D9).

| Job | Local | CI |
|---|---|---|
| `ui`, `multi-device`, `multi-device-ble-mock`, on your screen | `make e2e-headed` (`make e2e`) | – |
| `cli`, `ui`, `multi-device`, `multi-device-ble-mock`, in the e2e container | `make pr-gate-e2e` | `npm run gate:e2e:build-image`, `npm run gate:e2e:run` |
| `android` on virtual phones | `npm run test:e2e:android` | the same command, job `android-e2e` |
| a desktop process and your phone | `make e2e-phone` | – |

`scripts/ci-gate.mts` holds the gate steps. The e2e container starts one Xvfb
(glossary) for the run, and each app draws on it.

To run a spec on apps that run already (for example HMR apps), give them as
physical devices:

```sh
FINI_E2E_TEST_DEVICES=desktop=tcp:9224,phone=tcp:9223 \
  npx playwright test specs/e2e/multi-device/tests/<flow>.spec.ts \
  --config specs/e2e/playwright.config.ts --project=multi-device
```

## Not done yet

- The test with three phones (A to B to C).
- `bluetooth-channel-add-again` and `peer-sync-over-ble` on the virtual phones.
