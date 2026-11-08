# E2E tests

The e2e tests drive the real Fini app: the CLI, one desktop window, or two or
more apps that pair and sync. Playwright Test is the one runner. The decisions
are in `docs/adr/0013-e2e-test-devices.md`. The virtual Android phones are in
`docs/e2e-android.md`. What each project proves for each channel is in
"What each project proves for each channel" below.

## Parts

```
specs/e2e/
├── playwright.config.ts          projects; globalTeardown
├── e2e-project.ts                E2eProject (project names), E2eTag (test tags), E2eEnv (environment names)
├── fixtures/cli.ts               createCliClient(): the fini binary with a temp database
├── ui/fixtures.ts                tauriPage: one desktop app for each test
├── components/                   one class for each part of the screen (AddDeviceDialog, DevicePage, ...)
├── multi-device/
│   ├── test-device.ts            TestDevice; fixtures testDevices, testDeviceA, testDeviceB;
│   │                             options testDevicePlatforms, testDeviceChannel
│   ├── test-device-platform.ts   TestDevicePlatform, TestDeviceSource
│   ├── test-device-set.ts        TestDeviceSet: one provider for each test device
│   ├── run-root.ts               RUN_ROOT: the folder of all run folders
│   ├── providers/
│   │   ├── linux.ts              LinuxProvider: a virtual Linux test device (starts fini-app, unix socket bridge)
│   │   ├── android.ts            AndroidProvider: a virtual Android test device (the APK on a virtual phone)
│   │   ├── windows.ts            WindowsProvider: a virtual Windows test device (starts fini-app.exe, TCP bridge)
│   │   ├── tcp.ts                TcpProvider: the physical source, each platform (an app that runs already)
│   │   └── ble-mock-broker.ts    BleMockBroker: the mock radio of virtual Linux test devices
│   ├── phones/                   virtual phones (docs/e2e-android.md)
│   ├── global-teardown.ts        stops the virtual phones, if a test started them
│   ├── helpers/                  pairing and sync set-up (device-sync, personal-sync, ble-sync, ...)
│   └── tests/                    the multi-device specs
└── device-support/
    ├── supported-devices.ts      SUPPORTED_DEVICES: each platform and its channels; supportsChannel()
    └── tests/                    the device-support specs
```

## TestDevice

A `TestDevice` is one running Fini app that a test drives (glossary). It is
not the Fini `Device` model. A `TestDevice` has:

- `page`: a `TauriPage` for the screen.
- `invoke<T>(command, args)`: a call to a Tauri command.
- `platform`: `TestDevicePlatform.Linux`, `TestDevicePlatform.Android` or
  `TestDevicePlatform.Windows`.
- `source`: `TestDeviceSource.Virtual` (the harness starts it) or
  `TestDeviceSource.Physical` (it runs already; the harness connects to its
  TCP port). Each platform can have each source.
- `isVirtual(platform)`: true for a test device that the harness started on
  that platform.
- `shell(command)`: a shell command on the phone. Only a virtual Android test
  device has it.

A test gets its test devices from the fixture `testDevices` (by slug), or from
`testDeviceA` and `testDeviceB` (by position). The components in
`components/` take a `TauriPage`. They know nothing about test devices.

## How the test devices start

```
fixture testDevices (test-scoped)
└── testDeviceSet (worker-scoped)
    └── TestDeviceSet.fromEnv(testDevicePlatforms, testDeviceChannel)
        ├── FINI_E2E_TEST_DEVICES set? its entries, else the virtual test devices of testDevicePlatforms
        ├── a virtual platform not in FINI_E2E_VIRTUAL_PLATFORMS? → the test skips, with the reason
        ├── virtual Linux    → LinuxProvider     (and BleMockBroker when testDeviceChannel is Bluetooth)
        ├── virtual Windows  → WindowsProvider
        ├── virtual Android  → PhoneGroup.shared().serials(<number of virtual Android>) → AndroidProvider
        └── physical source  → TcpProvider
            each provider: prepare() → connect() → TestDevice waits for the page
```

- A spec names its test devices at the top of the file:
  `test.use({ testDevicePlatforms: { a: TestDevicePlatform.Android, b: TestDevicePlatform.Android } })`.
  The default is two virtual Linux test devices. A set has one test device or
  more.
- `FINI_E2E_TEST_DEVICES` gives other test devices in place of the ones of the
  spec: `slug=<platform>` (virtual) or `slug=<platform>@tcp:<port>` (physical),
  for example `a=linux,b=android@tcp:9223`.
- `FINI_E2E_VIRTUAL_PLATFORMS` lists the platforms that the job can start as
  virtual test devices (default: the desktop platform of the host, `linux` or
  `windows`). A test that needs another one skips.
  The physical source is always allowed.
- `dispose()` releases only what the provider made. A test device with the
  physical source stays as it is.
- Each run has a folder in `RUN_ROOT` (default `tmp/fini-e2e-multi-device/`).
  The folder holds the app data and the logs. The set deletes it after a
  passed run. With `FINI_E2E_KEEP=1`, or after a failed run, the set keeps it.

## The `testDeviceChannel` option

The channel that the test devices of a spec use, a `ChannelKind`
(`src/utils/channel.ts`). The default is `ChannelKind.Network`. With
`ChannelKind.Bluetooth`, virtual Linux test devices have the Network channel
off and use the `ble-gatt` mock radio (`BleMockBroker`). Virtual Android test
devices and the physical source use their own Bluetooth stack. It is a worker
option: set it at the top of a spec file, and Playwright starts a new worker
for that file. (Playwright has its own `channel` option, for the browser.)

## Projects

| Project | What it proves | Test devices | Specs |
|---|---|---|---|
| `cli` | the `fini` binary | none | `reminder-bridge.spec.ts`, `feature-plane-cli.spec.ts` |
| `ui` | one desktop app | one desktop app for each test | `ui/tests/` |
| `multi-device` | what Fini does with two or more devices | most specs: two virtual Linux over Network; a happy path names its own (`peer-sync-over-ble.spec.ts`: Bluetooth on the mock radio) | `multi-device/tests/` |
| `device-support` | that a platform and its channels work | each spec names its own (now: two virtual Android, or one virtual Windows) | `device-support/tests/` |

`multi-device` holds most tests. They run on the pair with the fewest
problems: two virtual Linux test devices over Network. Popular pairs and
chains on other platforms or channels are happy paths only.
`device-support` has the checks of each platform and channel, from the list
`SUPPORTED_DEVICES`. The list only says which channels each platform
supports. A check skips when a platform of its test devices does not support
its channel (`supportsChannel`). No project is made for each pair.

Playwright runs one worker, and the files in name order. A file that needs
the state of earlier files starts with `zz-`.

## Commands

CI calls only npm scripts. The Make targets are shortcuts for developers to
the same scripts (ADR-0013 D9).

| Job | Local | CI |
|---|---|---|
| `ui`, `multi-device`, on your screen | `make e2e-headed` (`make e2e`) | – |
| `cli`, `ui`, `multi-device`, in the e2e container | `make pr-gate-e2e` | `npm run gate:e2e:build-image`, `npm run gate:e2e:run` |
| `device-support` on virtual Android | `npm run test:e2e:android` | the same command, job `android-e2e` |
| `device-support` on a Windows host (specs for other platforms skip) | `npm run build:windows:e2e`, then `npm run test:e2e:device-support` (on Windows) | the same commands, job `desktop-bundle-windows` |
| a virtual Linux and your phone | `make e2e-phone` | – |
| the smoke tests (tag `E2eTag.Smoke`), in all projects | `npm run test:e2e:smoke` | – |

`scripts/ci-gate.mts` holds the gate steps. The e2e container starts one Xvfb
(glossary) for the run, and each app draws on it.

To run a spec on apps that run already (for example HMR apps), give them with
the physical source:

```sh
FINI_E2E_TEST_DEVICES=desktop=linux@tcp:9224,phone=android@tcp:9223 \
  npx playwright test specs/e2e/multi-device/tests/<flow>.spec.ts \
  --config specs/e2e/playwright.config.ts --project=multi-device
```

## What each project proves for each channel

The channels are in `docs/adr/0001-transport-neutral-peer-protocol.md` and
`docs/architecture/communication.md`.

| Layer | What it proves | Where it runs |
|---|---|---|
| Rust tests of `channel.service.rs` and `space-sync.service.rs` | The frames, the session and the sync rules. The same Rust code runs on each platform, so this proof is the same for each pair. | Each pull request (`npm run gate:be-unit`) |
| `multi-device`, `channel-order.spec.ts` | Two virtual Linux test devices on one network use the Network channel (QUIC over iroh). | Each pull request (Quality Gates) |
| `multi-device`, `peer-sync-over-ble.spec.ts` (`testDeviceChannel: ChannelKind.Bluetooth`) | Two virtual Linux test devices with the Network channel off, on the `ble-gatt` mock radio (`FINI_BLE_MOCK_BROKER`). The real code of `bluetooth-channel.service.rs` runs: the dial, the advertising and accepting side, the session, and the GATT roles. Both rows go green, and one quest syncs in both directions. It proves the protocol, the session and the UI, not the radio. | Each pull request (Quality Gates) |
| `device-support`, `bluetooth-pair-from-zero.spec.ts` | Two virtual Android test devices on one virtual Bluetooth medium (netsim): the Android Bluetooth stack, GATT and the APK. A new pair through Add Device until the Bluetooth row is green on both, with the 10 s standard for the listing. It does not prove the radio. | Local and CI (`npm run test:e2e:android`, job `android-e2e`; not a required check yet) |
| `device-support`, `android-app-start.spec.ts` | On a virtual Android test device (`testDeviceA` of the two phones): the app starts and makes the notification channel `fini.reminders`. | Local and CI (job `android-e2e`) |
| `device-support`, `windows-app-start.spec.ts` | On a virtual Windows test device: the desktop app shows its window and answers a command (`get_quests`). No channel yet: `SUPPORTED_DEVICES` lists none for Windows. | Local on Windows and CI (job `desktop-bundle-windows`) |
| The same specs with the physical source | `FINI_E2E_TEST_DEVICES=a=linux@tcp:9224,b=android@tcp:9223`: the same flows on a desktop and a phone with hardware radios. `bluetooth-channel-add-again.spec.ts` runs only like this: Bluetooth off and on again on a pair, both rows green in the 10 s standard (ADR-0011). | Local only |

CI does not test a hardware radio:

- GitHub-hosted runners have no Bluetooth radio. The mock radio and netsim
  prove the protocol, the session and GATT. No virtual Bluetooth medium
  (Bumble, `btvirt`, netsim) shows the faults of a hardware radio, for example radio
  interference or a fault of one adapter. These stay a test on physical
  devices.
- Virtual phones need KVM, which only the Linux runners have. Windows runners
  cannot run the Android emulator, so a Windows and Android pair has no CI
  path.

## Not done yet

- A test of hardware radios in CI (a device lab or a self-hosted runner). Now
  only the physical source on your desk tests them.
- The device-support checks for each channel: sync a quest in both
  directions, restart and connect again, unpair.
- Happy paths on more pairs and chains (for example Linux and Android over
  Network, or Windows → Android → Linux).
- A CI matrix from `SUPPORTED_DEVICES`, when a second platform can run in CI.
- `bluetooth-channel-add-again` and `peer-sync-over-ble` on the virtual phones.
