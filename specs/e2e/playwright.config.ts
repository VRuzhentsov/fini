import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '**/*.spec.ts',
  timeout: 30_000,
  reporter: 'list',
  // Browser e2e remains first-class for Fini (browser-first app). The current
  // `cli` project drives the Fini binary directly via stdio; future UI specs
  // can add their own project using Playwright's `page` fixture.
  use: { browserName: 'chromium' },
  projects: [{ name: 'cli', use: {} }],
});
