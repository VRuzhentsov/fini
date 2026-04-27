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
      testMatch: ['reminder-bridge.spec.ts'],
    },
    {
      name: 'ui',
      testMatch: ['ui/tests/**/*.spec.ts'],
      use: { mode: 'tauri' } as any,
    },
    {
      name: 'actors',
      testMatch: ['actors/tests/**/*.spec.ts'],
    },
  ],
});
