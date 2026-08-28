import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  timeout: 180_000,
  reporter: 'list',
  fullyParallel: false,
  workers: 1,
  projects: [
    {
      name: 'cli',
      testMatch: ['reminder-bridge.spec.ts', 'feature-plane-cli.spec.ts'],
    },
    {
      name: 'ui',
      testMatch: ['ui/tests/**/*.spec.ts'],
      use: { mode: 'tauri' } as any,
    },
    {
      name: 'actors',
      testMatch: ['actors/tests/**/*.spec.ts'],
      // Sim/BLE-transport specs need FINI_E2E_TRANSPORT set for the whole
      // actor process pool (the worker-scoped fixture spawns actors once,
      // shared by every test in this project) — see the 'actors-sim'/
      // 'actors-ble' projects below and `specs/e2e/transports.md`.
      testIgnore: [
        'actors/tests/peer-sync-over-sim.spec.ts',
        'actors/tests/unpair-and-rejoin-over-sim.spec.ts',
        'actors/tests/peer-sync-over-ble.spec.ts',
      ],
    },
    {
      // Opt-in: only runs when explicitly selected (`--project actors-sim`)
      // with FINI_E2E_TRANSPORT=sim in the environment. Never picked up by
      // an unfiltered `playwright test` run alongside the other projects,
      // since it needs network discovery genuinely disabled for its actors.
      name: 'actors-sim',
      testMatch: ['actors/tests/peer-sync-over-sim.spec.ts', 'actors/tests/unpair-and-rejoin-over-sim.spec.ts'],
    },
    {
      // Opt-in, same shape as 'actors-sim': only runs when explicitly
      // selected (`--project actors-ble`) with FINI_E2E_TRANSPORT=ble in
      // the environment. See `helpers/ble-sync.ts` and
      // `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`.
      name: 'actors-ble',
      testMatch: ['actors/tests/peer-sync-over-ble.spec.ts'],
    },
  ],
});
