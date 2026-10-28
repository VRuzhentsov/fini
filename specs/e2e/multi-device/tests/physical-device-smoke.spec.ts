import { expect, test } from '../test-device.ts';

/**
 * Proves that each kind of test device answers the same `invoke` surface: a
 * desktop process that the harness starts, and a physical device that it only
 * connects to over TCP (for example a phone through `adb forward`).
 *
 * Asserts only what is true for any two running apps: the identities exist
 * and are different. The pairs and data on a physical device are what that
 * device has, so this test asserts nothing about them.
 *
 * Run (phone prepared with `make android-debug-deploy` and `adb forward`):
 *   FINI_E2E_DEVICES=a=desktop-process,phone=tcp:9223 \
 *   npx playwright test physical-device-smoke \
 *     --config specs/e2e/playwright.config.ts --project=multi-device
 */
// Uses the test devices by position (testDeviceA, testDeviceB), not by slug,
// so the same spec runs on each mix of test devices.
test('both test devices answer the same invoke surface', async ({ testDeviceA, testDeviceB }) => {
  const identityA = await testDeviceA.invoke<{ device_id: string }>('device_get_identity');
  const identityB = await testDeviceB.invoke<{ device_id: string }>('device_get_identity');

  expect(identityA.device_id).toBeTruthy();
  expect(identityB.device_id).toBeTruthy();
  expect(identityB.device_id).not.toBe(identityA.device_id);
});
