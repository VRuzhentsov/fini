import { defineConfig } from '@playwright/test';
import { E2eProject } from './e2e-project.ts';

export default defineConfig({
  testDir: '.',
  timeout: 180_000,
  reporter: 'list',
  fullyParallel: false,
  workers: 1,
  // Stops the virtual phones only if a test started them (ADR-0013 D7).
  globalTeardown: './multi-device/global-teardown.ts',
  projects: [
    {
      name: E2eProject.Cli,
      testMatch: ['reminder-bridge.spec.ts', 'feature-plane-cli.spec.ts'],
    },
    {
      name: E2eProject.Ui,
      testMatch: ['ui/tests/**/*.spec.ts'],
      use: { mode: 'tauri' } as any,
    },
    {
      // What Fini does with two or more devices (ADR-0013 D11). By default two
      // virtual Linux test devices over Network; a spec names other platforms
      // or another channel with `test.use`.
      name: E2eProject.MultiDevice,
      testMatch: ['multi-device/tests/**/*.spec.ts'],
    },
    {
      // Checks that a platform and its channels work (ADR-0013 D11). Each spec
      // names its test devices with `test.use`.
      name: E2eProject.DeviceSupport,
      testMatch: ['device-support/tests/**/*.spec.ts'],
      // A phone boot (up to 360 s) is part of the first test.
      timeout: 900_000,
    },
  ],
});
