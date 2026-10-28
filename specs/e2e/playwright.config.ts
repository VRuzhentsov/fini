import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  timeout: 180_000,
  reporter: 'list',
  fullyParallel: false,
  workers: 1,
  // Stops the virtual phones only if a test started them (ADR-0012 D7).
  globalTeardown: './multi-device/global-teardown.ts',
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
      // Desktop processes over the Network channel.
      name: 'multi-device',
      testMatch: ['multi-device/tests/**/*.spec.ts'],
      // The BLE spec needs FINI_E2E_TRANSPORT=ble for all processes of the
      // worker; it runs in 'multi-device-ble-mock'. See `specs/e2e/transports.md`.
      testIgnore: ['multi-device/tests/peer-sync-over-ble.spec.ts'],
    },
    {
      // Desktop processes on the ble-gatt mock radio. Runs only when selected,
      // with FINI_E2E_TRANSPORT=ble. See `helpers/ble-sync.ts` and
      // `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in ble-gatt.
      name: 'multi-device-ble-mock',
      testMatch: ['multi-device/tests/peer-sync-over-ble.spec.ts'],
    },
    {
      // Two virtual phones on one virtual Bluetooth medium (ADR-0012). The
      // phones start on the first test that needs them.
      // FINI_E2E_TEST_DEVICES=a=tcp:9224,b=tcp:9223 runs it on physical devices.
      name: 'android-ble',
      testMatch: ['multi-device/tests/bluetooth-pair-from-zero.spec.ts'],
      timeout: 600_000,
      use: { testDeviceKinds: 'a=android-phone,b=android-phone', phoneCount: 2 } as any,
    },
  ],
});
