import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './ui/tests',
  timeout: 180_000,
  reporter: 'list',
  fullyParallel: false,
  workers: 1,
  projects: [
    {
      name: 'tauri',
      testMatch: '**/*.spec.ts',
      use: { mode: 'tauri' } as any,
    },
  ],
});
