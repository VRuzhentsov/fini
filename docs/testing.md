# Testing

The test surfaces of Fini, how to run them, and how to write each kind of
test. The e2e harness is in [`e2e.md`](e2e.md); e2e on virtual Android phones
is in [`e2e-android.md`](e2e-android.md); a physical Android phone is in
[`android.md`](android.md). Where CI runs each target is in [`ci.md`](ci.md).

## Test surfaces

| Surface | Runner | Entry point | Location |
|---|---|---|---|
| FE unit | Jest + `@vue/test-utils` + jsdom | local: `npm run test:unit`; CI: job `fe-unit-tests` (`ci.yml`), `npm run test:unit` in the `linux` `build` image | `src/spec/**/*.spec.ts` |
| BE unit | `cargo test` | local: `npm run gate:be-unit` (local shortcut `make pr-gate-be-unit`); CI: job Quality Gates (`quality-gates.yml`), `cargo test` in the `linux` `build` image | inline `#[cfg(test)]` modules in `src-tauri/src/**/*.rs` |
| E2E single-app UI | Playwright + `@srsholmes/tauri-playwright` | local: `make e2e-headed`; CI: job Quality Gates (`quality-gates.yml`), `npm run gate:e2e:run` (project `ui`) | `specs/e2e/ui/tests/*.spec.ts` |
| E2E multi-device | Playwright + the `testDevices` fixture (ADR-0013) | local: `make e2e-headed`; CI: job Quality Gates (`quality-gates.yml`), `npm run gate:e2e:run` (project `multi-device`) | `specs/e2e/multi-device/tests/*.spec.ts` |
| E2E device support (now: Android) | project `device-support`, virtual Android test devices | `npm run test:e2e:android`, local and CI (job `android-e2e`) | `specs/e2e/device-support/tests/*.spec.ts` |
| E2E CLI-only | Playwright + `createCliClient()` | project `cli` in `specs/e2e/playwright.config.ts` | `specs/e2e/*.spec.ts` (e.g. `reminder-bridge.spec.ts`) |
| Android on a physical phone | manual device automation | [`android.md`](android.md) and the `fini-android-testing` skill | — |

CI parity: the workflows build the same `containers/` stages with the container engine of CI (`.github/actions/container-stage`; `container.md`, "Container engine") and run the same commands in them: `npm run test:unit`, `cargo test`, `npm run gate:e2e:run` and `npm run test:e2e:android`. On a developer's computer, `npm run gate:*` and `npm run test:e2e:android` (`scripts/ci-gate.mts`, `scripts/e2e-android.mts`) build and run the same stages with the local container engine; the `make pr-gate-*` targets are local shortcuts to them.

## Choose the test category

- Pure Vue logic, store, or pure component behavior → **FE unit (Jest)**.
- Pure Rust logic, DB migration, service-layer behavior → **BE unit (`#[cfg(test)]`)**.
- Single Fini app, real GUI flow, single user view → **E2E single-app UI** (`specs/e2e/ui/`).
- Two or more Fini apps (device pairing, personal/space sync, multi-device behavior) → **E2E multi-device** (`specs/e2e/multi-device/`), on two virtual Linux test devices over Network. A happy path on another pair or channel names it at the top of the file (`test.use({ testDevicePlatforms, testDeviceChannel })`).
- That a platform and its channels work (the Android app start, Bluetooth between two Android stacks) → project `device-support` (`specs/e2e/device-support/`).
- CLI behavior, or the bridge to the Rust side with no GUI → **E2E CLI** (`specs/e2e/*.spec.ts`, project `cli`).
- Android-only behavior on a physical phone → [`android.md`](android.md).

If a behavior fits more than one surface, pick the cheapest to run and the most isolated. Multi-device is expensive, and `device-support` on virtual Android the most expensive — use them only for behaviors that need two apps.

A behavior that does not fit the surfaces above needs a harness change, not a test change: new Makefile targets, npm scripts, Playwright projects, or CI jobs ([`development.md`](development.md), "Repo automation"). Add the new surface to this file once it lands.

## Running tests

### Frontend unit

```sh
npm run test:unit
```

Underlying command: `jest --runInBand`. `npm run gate:fe-unit` (local shortcut `make pr-gate-fe-unit`) runs the same suite in the `linux` `build` image (`containers/linux/Containerfile`); CI runs `npm run test:unit` in the same image.

### Backend unit

```sh
npm run gate:be-unit
```

This runs `cargo test --manifest-path src-tauri/Cargo.toml --features cli-plane,ui-plane,desktop-updater` in the `linux` `build` image (`containers/linux/Containerfile`); `gate:be-compile` only compiles the tests. To iterate locally without containers, run `cargo test --manifest-path src-tauri/Cargo.toml` directly.

### E2E — visible local run (single-app UI + multi-device)

```sh
make e2e-headed
```

Builds `fini-app` with `--features ui-plane,desktop-updater,devtools`, `fini` with `--features cli-plane` and the mock broker in `src-tauri/target/debug-e2e` (`scripts/e2e-build.mts desktop`), then runs the projects `ui` and `multi-device` (`scripts/e2e-headed.mts`). The `testDevices` fixture starts the virtual Linux test devices. `make e2e` is an alias for `make e2e-headed`.

### E2E — containerized CI parity

```sh
npm run gate:e2e:build-image && npm run gate:e2e:run
```

The `linux` `e2e` image starts the virtual Linux test devices. CI runs `npm run gate:e2e:run` in the job Quality Gates (`quality-gates.yml`). On a developer's computer, `make pr-gate-e2e` (or `make e2e-ci`) runs the same two steps.

### E2E — `device-support` on virtual Android

```sh
npm run test:e2e:android [-- <playwright args>]
```

Needs the x86_64 APK with devtools first: `npm run build:android:e2e`. The image, when the phones start, the logs and the rules for the specs: [`e2e-android.md`](e2e-android.md).

### Targeting a single Playwright project

The projects in `specs/e2e/playwright.config.ts`: `cli`, `ui`, `multi-device` (what Fini does with two or more devices; by default two virtual Linux test devices over Network), `device-support` (that a platform and its channels work). See `e2e.md`, "Projects". Pass `--project=<name>` to `npx playwright test --config specs/e2e/playwright.config.ts`.

### Running only the smoke tests

```sh
npm run test:e2e:smoke
```

Runs the tests with the tag `E2eTag.Smoke` (`@smoke`) in all projects: quick checks that the harness starts the test devices and the app answers.

### No generic targets

`make test`, `make lint`, `make check` do not exist. Use the named commands above. If the desired check has no Makefile target, say so explicitly.

## Drive the running apps

To make the running apps (desktop and phone, HMR) do something, use one of these two ways. Pick the way for the task:

1. **One manual action at a time** — to do something once: unblock a flow, read a state, see one screen. Send one action with the devtools bridge (`node .agents/skills/fini-android-testing/devtools-bridge.mjs <port> "<js>"`; see [`android.md`](android.md), "The devtools bridge"), read what the screen shows now, then decide the next action. One call is one action: one click, one read, or one command.
2. **A Playwright spec built from components** — for a flow with more than one step, a flow to do again, or a flow whose times count. Write the flow in `specs/e2e/` from the components in `specs/e2e/components/`, and run it against the running apps with the physical source:

   ```sh
   FINI_E2E_TEST_DEVICES=desktop=linux@tcp:9224,phone=android@tcp:9223 \
   PLAYWRIGHT_JSON_OUTPUT_NAME=~/.cache/agents/<workflow>/e2e.json \
   npx playwright test specs/e2e/multi-device/tests/<flow>.spec.ts \
     --config specs/e2e/playwright.config.ts --project=multi-device --reporter=list,json
   ```

   Step times go in test annotations (`test.info().annotations`); read them from the JSON report.

Components:

- A component is a class around one part of the screen, made from a `TauriPage`: `AddDeviceDialog`, `DevicePage`, `ChannelRow`. It knows nothing about test devices.
- One method is one action that a person does (`openSearch(channel)`, `pair(hostname)`, `unlinkDevice()`), and it returns when the screen shows the result. Wait with Playwright (`waitForSelector`, `waitForFunction`) on a `data-*` hook, for example `[data-pair-step="looking"]`.
- When a step has no hook, add one to the Vue component (`data-testid`, or a `data-*` attribute that names the state; `naming.md`), then wait on it.
- When a flow needs a part of the screen that has no component, add a component for it in `specs/e2e/components/`, then use it in the spec.

## Writing FE unit tests

Layout: `src/spec/<group>/<name>.spec.ts`. The Jest config (`jest.config.cjs`) restricts `roots` to `src/spec` and `testMatch` to `**/*.spec.ts`. Setup file: `src/spec/jest.setup.ts`. Path alias: `@/*` → `src/*`.

Mock `@tauri-apps/api/core` and use Pinia in tests. Skeleton from `src/spec/stores/device.store.spec.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import { createPinia, setActivePinia } from "pinia";
import { useDeviceStore } from "../../stores/device";

jest.mock("@tauri-apps/api/core", () => ({
  invoke: jest.fn(),
}));

describe("device store sync status", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (invoke as unknown as jest.Mock).mockReset();
  });

  it("updates last synced timestamp from status response", async () => {
    (invoke as unknown as jest.Mock).mockResolvedValueOnce({ /* … */ });
    const store = useDeviceStore();
    await store.refreshSpaceSyncStatus("peer-1");
    expect(store.getLastSyncedAt("peer-1")).toBe("2026-04-07T13:20:00Z");
  });
});
```

Conventions:

- Mount components with `@vue/test-utils` `mount()` when the test needs DOM behavior; otherwise prefer pure store/composable tests.
- Reset every `invoke` mock in `beforeEach`. Tests run with `--runInBand`, but mock state still leaks across `it` blocks unless explicitly reset.
- Match the file path of the production code: `src/stores/device.store.ts` ↔ `src/spec/stores/device.store.spec.ts`, `src/views/settings/DeviceView.vue` ↔ `src/spec/views/DeviceView.spec.ts`.
- When a test mocks an entire module (`jest.mock`), re-export all named constants and functions the component under test imports from that module — omitting them silently produces `undefined` at runtime.

### Vue render states

When a change adds or changes render flags ([`code-style.md`](code-style.md), "Centralize render decisions in `renderFlags`"):

- Add or update component tests for both visible and hidden states when the condition is product/platform behavior.
- Prefer assertions on user-visible labels or test IDs, not internal computed names.
- Include at least one test that would fail if the section rendered unconditionally.

## Writing BE unit tests

Layout: inline `#[cfg(test)] mod tests` at the bottom of the source file. Existing examples live next to the production code in `src-tauri/src/services/db.service.rs`, `src-tauri/src/commands/quest.commands.rs`, `src-tauri/src/repositories/quest.repository.rs`, `src-tauri/src/services/settings.service.rs`.

For DB-touching tests, use the `temp_db_path(label)` helper in `src-tauri/src/services/db.service.rs`:

```rust
#[cfg(test)]
pub fn temp_db_path(label: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    std::env::temp_dir().join(format!("fini-{label}-{unique}.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_space_ids_exist_after_migration() {
        let db_path = temp_db_path("built-in-space-ids-exist-after-migration");
        let mut conn = open_db_at_path(&db_path);
        // assertions …
        let _ = std::fs::remove_file(db_path);
    }
}
```

Conventions:

- Keep the `#[cfg(test)]` module at the bottom of the file under test, never in a sibling file.
- Pass a unique `label` per test to `temp_db_path` so parallel runs do not collide.
- Clean up the temp DB at the end of the test (`std::fs::remove_file`) when the test owns the file.
- Migration tests: [`db.md`](db.md), "Schema migrations".

## Writing single-app UI e2e

Layout: `specs/e2e/ui/tests/<feature>.spec.ts`. Fixture: `specs/e2e/ui/fixtures.ts` exports `test` and `expect` from `createTauriTest` with a socket under the task-scoped E2E root. The test receives a `tauriPage` that drives the real Fini window.

App data is isolated under the task-scoped `FINI_E2E_ROOT` (default: the repository's ignored `tmp/fini-e2e-ui/`) so the user's normal data directory is untouched. The app is launched with `--features devtools`.

Skeleton from `specs/e2e/ui/tests/context-menu-accordion.spec.ts`:

```ts
import { test, expect } from '../fixtures.ts';

test('wide: Move to space opens as in-place accordion, not a flyout', async ({ tauriPage }) => {
  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await tauriPage.waitForSelector('[data-testid="chat-input"]', 30_000);
  // … interact, assert via tauriPage.evaluate / waitForFunction …
});
```

Conventions:

- Prefer `[data-testid="…"]` selectors over CSS class chains; class chains break under design changes.
- Use `tauriPage.waitForSelector` / `waitForFunction` instead of fixed `setTimeout`.
- For non-trivial DOM mutation (e.g. setting `<textarea>` value programmatically), drop into `tauriPage.evaluate` with a small inline helper, as `fillTextarea` does in the file above.
- File names use lowercase kebab-case ending in `.spec.ts`.

## Writing multi-device e2e

Layout: `specs/e2e/multi-device/tests/<feature>.spec.ts`. Fixture: `specs/e2e/multi-device/test-device.ts` gives `testDevices` (by slug), `testDeviceA` and `testDeviceB` (by position). A `TestDevice` is one running Fini app that the test drives (glossary, ADR-0013); it is not the Fini `Device` model. Each one has `page` (a `TauriPage`), `invoke<T>(command, args)`, `platform` (`TestDevicePlatform`) and `source` (`TestDeviceSource`: virtual or physical).

How the test devices start (`FINI_E2E_TEST_DEVICES`, providers, run folders): [`e2e.md`](e2e.md).

Helpers live in `specs/e2e/multi-device/helpers/`:

- `device-sync.ts` — `pairTestDevices(...)`: pairs the test devices (through Add Device with `pairInUi`).
- `personal-sync.ts` — `syncPersonalSpace(...)`: maps the Personal space of two paired test devices.
- `ble-sync.ts` — `pairTestDevicesOverBluetooth(...)` for specs with `testDeviceChannel: ChannelKind.Bluetooth`.
- `dom.ts` — shared DOM checks.
- `teardown.ts` — `resetTestDevicesUi(...)` runs after every test.

Skeleton from `specs/e2e/multi-device/tests/two-test-devices.spec.ts`:

```ts
import { test, expect } from '../test-device.ts';
import { E2eTag } from '../../e2e-project.ts';

interface DeviceIdentity { device_id: string; hostname: string; }

test('runner controls two isolated test devices with different identities', { tag: E2eTag.Smoke }, async ({ testDeviceA, testDeviceB }) => {
  const identityA = await testDeviceA.invoke<DeviceIdentity>('device_get_identity');
  const identityB = await testDeviceB.invoke<DeviceIdentity>('device_get_identity');
  expect(identityA.device_id).not.toBe(identityB.device_id);
});
```

Conventions:

- Use a helper from `helpers/` instead of re-implementing pairing or sync inline. If a helper is missing, add it next to the others rather than embedding the setup in the test.
- Prefix late-running test files with `zz-` (see `zz-personal-space-live-quest-sync.spec.ts`) when the test depends on state established by earlier tests in the same project — Playwright runs files in lexical order with `fullyParallel: false, workers: 1`.
- Talk to the Rust side through `testDevice.invoke<T>(command, args)`; use `testDevice.page.evaluate` only for DOM checks.
- A spec that needs two Bluetooth stacks skips when a test device is virtual Linux (`testDevice.isVirtual(TestDevicePlatform.Linux)`; see `bluetooth-pair-from-zero.spec.ts`).
- Use the consts (`TestDevicePlatform`, `TestDeviceSource`, `ChannelKind`), never their string values.
- Mark a group of tests with a Playwright tag, not with a word in the file name. Name the file for what it checks (`two-test-devices.spec.ts`, not `multi-device-smoke.spec.ts`). Give a quick check that the harness and the app answer `{ tag: E2eTag.Smoke }` (`specs/e2e/e2e-project.ts`); a new tag goes into `E2eTag` first, never as a string in a spec. Run them with `npm run test:e2e:smoke` (`--grep @smoke`), or leave them out with `--grep-invert`.
- With virtual Android test devices, obey the rules in [`e2e-android.md`](e2e-android.md), "Rules for the specs".

## Writing CLI e2e

Layout: `specs/e2e/<feature>.spec.ts` matched by the `cli` Playwright project. Fixture: `specs/e2e/fixtures/cli.ts` exports `createCliClient()`. Each client gets a fresh `mkdtemp` directory and a temp SQLite DB path passed to the binary as `FINI_DB_PATH`. Commands run with `--json` and stdout is parsed.

Skeleton from `specs/e2e/reminder-bridge.spec.ts`:

```ts
import { test, expect } from '@playwright/test';
import { CliClient, createCliClient } from './fixtures/cli.ts';

let cli: CliClient;

test.beforeEach(() => { cli = createCliClient(); });
test.afterEach(()  => { cli.close(); });

test('setting due+due_time auto-creates a reminder row', async () => {
  const quest = cli.run<Quest>(['quest', 'create', '--title', 'reminder bridge']);
  // … cli.run([...]) for further commands, parse JSON, assert …
});
```

Conventions:

- One `CliClient` per test; close it in `afterEach` so the temp DB is removed.
- Use `--json` mode (the fixture already prepends `--json`); never parse human output.
- Tests run with `TZ=UTC` so wall-clock fields are stable.
- CLI command semantics and argument shape: `specs/cli/README.md` and [`development.md`](development.md), "Fini CLI".
