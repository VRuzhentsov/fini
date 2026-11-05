import { expect, test } from '../test-device.ts';

/**
 * The Fini app starts on an Android phone and makes its notification channel,
 * so that reminders can show (`CHANNEL_ID` in src-tauri/src/services/notification.rs).
 * It runs before the Bluetooth test in the `android` project (files run in
 * name order); the phones are up already.
 */

const PACKAGE = process.env.FINI_E2E_PACKAGE ?? 'com.fini.app.debug';
const NOTIFICATION_CHANNEL = 'fini.reminders';
const CHANNEL_TIMEOUT_MS = 20_000;

test('the app starts and makes its notification channel', async ({ testDeviceA }) => {
  test.skip(testDeviceA.kind !== 'android-phone', 'needs a virtual phone');

  expect((await testDeviceA.shell(`pidof ${PACKAGE}`)).trim()).not.toBe('');
  await expect
    .poll(async () => (await testDeviceA.shell('dumpsys notification')).includes(NOTIFICATION_CHANNEL), {
      timeout: CHANNEL_TIMEOUT_MS,
    })
    .toBe(true);
});
