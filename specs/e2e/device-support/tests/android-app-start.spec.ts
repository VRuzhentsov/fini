import { expect, test, TestDevicePlatform } from '../../multi-device/test-device.ts';

/**
 * The Fini app starts on Android and makes its notification channel, so that
 * reminders can show (`CHANNEL_ID` in src-tauri/src/services/notification.rs).
 * It runs before the Bluetooth check (files run in name order) with the same
 * test devices, so the phones are up already.
 */

const PACKAGE = process.env.FINI_E2E_PACKAGE ?? 'com.fini.app.debug';
const NOTIFICATION_CHANNEL = 'fini.reminders';
const CHANNEL_TIMEOUT_MS = 20_000;

test.use({ testDevicePlatforms: { a: TestDevicePlatform.Android, b: TestDevicePlatform.Android } });

test('the app starts and makes its notification channel', async ({ testDeviceA }) => {
  test.skip(!testDeviceA.isVirtual(TestDevicePlatform.Android), 'needs a virtual Android test device (it uses shell)');

  expect((await testDeviceA.shell(`pidof ${PACKAGE}`)).trim()).not.toBe('');
  await expect
    .poll(async () => (await testDeviceA.shell('dumpsys notification')).includes(NOTIFICATION_CHANNEL), {
      timeout: CHANNEL_TIMEOUT_MS,
    })
    .toBe(true);
});
