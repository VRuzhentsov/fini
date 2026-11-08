import { expect, test } from '../../multi-device/test-device.ts';
import { SUPPORTED_DEVICES } from '../supported-devices.ts';

/**
 * The Fini app starts on each platform of `SUPPORTED_DEVICES`: the window shows
 * the app (the harness waits for the navigation), and the Rust side answers a
 * command with its database. Then the check of the platform (`appStart`), if
 * it has one. A platform that this job cannot start skips with the reason.
 */

for (const device of SUPPORTED_DEVICES) {
  // `testDevicePlatforms` is a worker option, and Playwright does not let
  // `test.use` change it in a describe group. Thus each platform gets its own
  // `test` with its test devices.
  const platformTest = test.extend({
    testDevicePlatforms: [device.appStartTestDevices ?? { a: device.platform }, { scope: 'worker' }],
  });

  platformTest.describe(device.platform, () => {
    platformTest('the app starts', async ({ testDeviceA }) => {
      platformTest.skip(!testDeviceA.isVirtual(device.platform), `needs a virtual ${device.platform} test device`);

      expect(await testDeviceA.page.evaluate<boolean>(`Boolean(document.querySelector('nav.nav'))`)).toBe(true);
      expect(Array.isArray(await testDeviceA.invoke<unknown[]>('get_quests'))).toBe(true);
      await device.appStart?.(testDeviceA);
    });
  });
}
