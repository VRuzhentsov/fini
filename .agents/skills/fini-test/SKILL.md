---
name: fini-test
description: "Single source of truth for testing the Fini app: how to run unit, integration, and end-to-end tests, and how to write new ones. Use when the user asks to run tests, write a test, add coverage, debug a failing test, choose a test category, set up a multi-actor scenario, or understand fixtures and harnesses. For Android-only behavior, load `fini-android-testing` instead. For CLI semantics referenced from CLI e2e tests, also load `fini-cli`. For Makefile/CI/script changes around tests, also load `fini-scripting`."
---

# Fini Testing

Use this skill to run, write, and reason about tests in the Fini repo. It is a documentation skill: it teaches conventions and quotes the existing harnesses, but it does not invent new ones. For harness or automation changes, route through `fini-scripting`.

## Goal

Make every test claim defensible:

1. Pick the correct test surface for the behavior under test.
2. Run the test through the documented entry point.
3. Cite the exact command and the artifacts the run produced.

## Test surfaces

| Surface | Runner | Entry point | Location |
|---|---|---|---|
| FE unit | Jest + `@vue/test-utils` + jsdom | `npm run test:unit` (no Makefile wrapper) | `src/spec/**/*.spec.ts` |
| BE unit | `cargo test` | `make pr-gate-be-unit` | inline `#[cfg(test)]` modules in `src-tauri/src/**/*.rs` |
| E2E single-actor UI | Playwright + `@srsholmes/tauri-playwright` | local: `make e2e-headed`; CI: `make pr-gate-e2e-run` (project `ui`) | `specs/e2e/ui/tests/*.spec.ts` |
| E2E multi-actor | Playwright + custom socket fixtures | local: `make e2e-headed`; CI: `make pr-gate-e2e` chain (project `actors`) | `specs/e2e/actors/tests/*.spec.ts` |
| E2E CLI-only | Playwright + `createCliClient()` | project `cli` in `specs/e2e/playwright.config.ts` | `specs/e2e/*.spec.ts` (e.g. `reminder-bridge.spec.ts`) |
| Android | manual device automation | route to `fini-android-testing` | covered by that skill |

CI parity: `.github/workflows/ci.yml` runs `pr-gate-fe-unit`, `pr-gate-be-compile`, `pr-gate-be-unit`, and the full `pr-gate-e2e-*` chain.

## Choose the right test category

Decide before writing the test:

- Pure Vue logic, store, or pure component behavior → **FE unit (Jest)**.
- Pure Rust logic, DB migration, service-layer behavior → **BE unit (`#[cfg(test)]`)**.
- Single Fini app, real GUI flow, single user view → **E2E single-actor UI** (`specs/e2e/ui/`).
- Two or more Fini instances (device pairing, personal/space sync, multi-device behavior) → **E2E multi-actor** (`specs/e2e/actors/`).
- CLI behavior or backend bridge with no GUI need → **E2E CLI** (`specs/e2e/*.spec.ts`, project `cli`).
- Android-only behavior → load `fini-android-testing` instead.

If a behavior fits more than one surface, pick the cheapest to run and the most isolated. Multi-actor is the most expensive — reserve it for behaviors that genuinely require two instances.

## Running tests

### Frontend unit

```sh
npm run test:unit
```

Underlying command: `jest --runInBand`. `npm run test:unit` has no Makefile wrapper today; use the npm script directly. CI runs the same suite via `make pr-gate-fe-unit`, which builds the `fe-unit-test` Dockerfile stage.

### Backend unit

```sh
make pr-gate-be-unit
```

This depends on `make pr-gate-be-compile`, then runs the `be-unit-test` Dockerfile stage which executes `cargo test --manifest-path src-tauri/Cargo.toml`. To iterate locally without containers, run `cargo test --manifest-path src-tauri/Cargo.toml` directly.

### E2E — visible local run (single-actor UI + multi-actor)

```sh
make e2e-headed
```

Builds `fini-app` with `--features ui-plane,devtools` and `fini` with `--features cli-plane,devtools`, then executes `npx playwright test --config specs/e2e/playwright.config.ts --project ui --project actors`. The Playwright fixtures spawn the real app processes. `make e2e` is an alias for `make e2e-headed`.

### E2E — containerized CI parity

```sh
make pr-gate-e2e
```

Runs the chain `pr-gate-e2e-build-dev-runner` → `pr-gate-e2e-run`. The dev-runner image owns actor startup and sockets. `make e2e-ci` is a shortcut for the same containerized E2E flow.

### Targeting a single Playwright project

The Playwright config (`specs/e2e/playwright.config.ts`) exposes three projects:

```ts
projects: [
  { name: 'cli',    testMatch: ['reminder-bridge.spec.ts'] },
  { name: 'ui',     testMatch: ['ui/tests/**/*.spec.ts'], use: { mode: 'tauri' } as any },
  { name: 'actors', testMatch: ['actors/tests/**/*.spec.ts'] },
],
```

Pass `--project cli|ui|actors` when invoking `npx playwright test --config specs/e2e/playwright.config.ts` directly.

### Do not invent targets

`make test`, `make lint`, `make check` do not exist. Use the named commands above. If the desired check has no Makefile target, say so explicitly.

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
- Match the file path of the production code: `src/stores/device.ts` ↔ `src/spec/stores/device.store.spec.ts`, `src/views/DeviceView.vue` ↔ `src/spec/views/DeviceView.spec.ts`.

## Writing BE unit tests

Layout: inline `#[cfg(test)] mod tests` at the bottom of the source file. Existing examples live next to the production code in `src-tauri/src/services/db.rs`, `src-tauri/src/services/quest.rs`, `src-tauri/src/services/settings.rs`, `src-tauri/src/services/mcp.rs`, `src-tauri/src/services/device_connection/runtime.rs`.

For DB-touching tests, use the `temp_db_path(label)` helper in `src-tauri/src/services/db.rs`:

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

## Writing single-actor UI e2e

Layout: `specs/e2e/ui/tests/<feature>.spec.ts`. Fixture: `specs/e2e/ui/fixtures.ts` exports `test` and `expect` from `createTauriTest({ tauriCommand, mcpSocket: '/var/tmp/fini-playwright.sock' })`. The test receives a `tauriPage` that drives the real Fini window.

App data is isolated under `FINI_APP_DATA_DIR=/var/tmp/fini-e2e-ui` so the user's normal data directory is untouched. The app is launched with `--features devtools`.

Skeleton from `specs/e2e/ui/tests/context-menu-submenu-hover.spec.ts`:

```ts
import { test, expect } from '../fixtures.ts';

test('context-menu submenu stays open during hover-to-submenu cursor traversal', async ({ tauriPage }) => {
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

## Writing multi-actor e2e

Layout: `specs/e2e/actors/tests/<feature>.spec.ts`. Fixture: `specs/e2e/actors/fixtures.ts` exposes `actorA`, `actorB`, and a generic `actors` map. Each actor is a separate Fini instance with its own `FINI_APP_DATA_DIR` and its own Unix socket; the runner connects via `PluginClient(socketPath)` and exposes `actor.page` (a `TauriPage`) and `actor.invoke<T>(command, args)` (a typed Tauri command call).

Actor processes are spawned by the worker-scoped fixture in `specs/e2e/actors/fixtures.ts`. The runner discovers actors through environment variables:

- `FINI_E2E_ACTORS` — comma-separated slugs (default `actor-a,actor-b`).
- `FINI_E2E_SOCKET_DIR` — directory where each actor's `<slug>.sock` appears (default `/var/run/fini-e2e`).

Helpers live in `specs/e2e/actors/helpers/`:

- `device-sync.ts` — `ensureSyncedActors(...)` for paired-device setup.
- `personal-sync.ts` — `ensurePersonalSpaceSync(...)` for personal-space replication setup.
- `dom.ts` — shared DOM probes.
- `teardown.ts` — `resetActorsUi(...)` runs after every test in the fixture's `finally`.

Skeleton from `specs/e2e/actors/tests/multi-actor-smoke.spec.ts`:

```ts
import { test, expect } from '../fixtures.ts';

interface DeviceIdentity { device_id: string; hostname: string; }

test('runner controls two isolated actors with distinct identities', async ({ actorA, actorB }) => {
  await actorA.page.waitForSelector('nav.nav', 30_000);
  await actorB.page.waitForSelector('nav.nav', 30_000);

  const identityA = await actorA.invoke<DeviceIdentity>('device_connection_get_identity');
  const identityB = await actorB.invoke<DeviceIdentity>('device_connection_get_identity');

  expect(identityA.device_id).not.toBe(identityB.device_id);
  expect(identityA.hostname).toBe('actor-a');
  expect(identityB.hostname).toBe('actor-b');
});
```

Conventions:

- Use a helper from `helpers/` instead of re-implementing pairing or sync inline. If a helper is missing, add it next to the others rather than embedding the setup in the test.
- Prefix late-running test files with `zz-` (see `zz-personal-space-live-quest-sync.spec.ts`) when the test depends on state established by earlier tests in the same project — Playwright runs files in lexical order with `fullyParallel: false, workers: 1` per `specs/e2e/playwright.config.ts`.
- Talk to the backend through `actor.invoke<T>(command, args)` for Tauri commands; reach for `actor.page.evaluate` only for DOM probes the page exposes naturally.
- Never hard-code socket paths — read them via the fixture; the fixture computes `path.join(FINI_E2E_SOCKET_DIR, '<slug>.sock')`.

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
- For CLI command semantics or argument shape, load `fini-cli` — that skill owns the CLI surface.

## Evidence requirement

Before claiming a test passes, present:

1. **Command** — the exact command invoked, including project flags or env vars.
2. **Outcome** — observed exit code or pass/fail summary line from the runner.
3. **Artifacts** — for e2e, the `test-results/` directory path produced by Playwright (videos, traces, screenshots); for unit tests, the runner's printed test count.
4. **Limits** — any test skipped, any harness not exercised, any flake suspected.

Failures must include the same four items plus the failing test name and the assertion message.

## Adding a new test category or harness

If a behavior does not fit the surfaces above, it is a harness change, not a test change. Stop and load `fini-scripting` before adding new Makefile targets, npm scripts, Playwright projects, or CI jobs. Document the new surface here once it lands so this skill stays the single source of truth.
