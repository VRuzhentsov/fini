import { expect, test, TestDevicePlatform } from '../../multi-device/test-device.ts';

/**
 * The Fini desktop app starts on Windows: the window shows the app (the
 * harness waits for the navigation), and the Rust side answers a command with
 * its database. The minimum check of the platform; its channels come later.
 */

test.use({ testDevicePlatforms: { a: TestDevicePlatform.Windows } });

test('the desktop app starts and answers a command', async ({ testDeviceA }) => {
  test.skip(!testDeviceA.isVirtual(TestDevicePlatform.Windows), 'needs a virtual Windows test device');

  expect(await testDeviceA.page.evaluate<boolean>(`Boolean(document.querySelector('nav.nav'))`)).toBe(true);
  expect(Array.isArray(await testDeviceA.invoke<unknown[]>('get_quests'))).toBe(true);
});
