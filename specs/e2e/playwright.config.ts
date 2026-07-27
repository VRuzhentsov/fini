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
      // Sim-transport specs need FINI_E2E_TRANSPORT=sim set for the whole
      // actor process pool (the worker-scoped fixture spawns actors once,
      // shared by every test in this project) — see the 'actors-sim'
      // project below and `specs/e2e/transports.md`.
      testIgnore: ['actors/tests/peer-sync-over-sim.spec.ts'],
    },
    {
      // Opt-in: only runs when explicitly selected (`--project actors-sim`)
      // with FINI_E2E_TRANSPORT=sim in the environment. Never picked up by
      // an unfiltered `playwright test` run alongside the other projects,
      // since it needs network discovery genuinely disabled for its actors.
      name: 'actors-sim',
      testMatch: ['actors/tests/peer-sync-over-sim.spec.ts'],
    },
  ],
});
