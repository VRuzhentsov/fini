import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 120_000,
  reporter: 'list',
  fullyParallel: false,
  workers: 1,
  use: {
    mode: 'tauri' as any,
  },
});
