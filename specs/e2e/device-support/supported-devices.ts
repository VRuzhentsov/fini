// The platforms that Fini supports, and the channels on each (ADR-0013 D11).
// The project `device-support` checks each entry: `tests/app-start.spec.ts`
// runs for each platform, and a channel check runs where `supportsChannel` is
// true. A spec that checks a channel names its own test devices with `test.use`.

import { expect } from '@playwright/test';
import { ChannelKind } from '../../../src/utils/channel.ts';
import type { TestDevice } from '../multi-device/test-device.ts';
import { TestDevicePlatform } from '../multi-device/test-device-platform.ts';

export interface SupportedDevice {
  platform: TestDevicePlatform;
  channels: ChannelKind[];
  /**
   * The test devices of the app-start check, slug → platform. Without it, one
   * test device of `platform`. `testDeviceA` is the one that the check uses.
   */
  appStartTestDevices?: Record<string, TestDevicePlatform>;
  /** The check of the app start on this platform, after the common check. */
  appStart?: (testDevice: TestDevice) => Promise<void>;
}

const ANDROID_PACKAGE = process.env.FINI_E2E_PACKAGE ?? 'com.fini.app.debug';
/** `CHANNEL_ID` in src-tauri/src/services/notification.rs. */
const ANDROID_NOTIFICATION_CHANNEL = 'fini.reminders';
const ANDROID_CHANNEL_TIMEOUT_MS = 20_000;

/** The app process runs, and it makes its notification channel, so that reminders can show. */
async function androidAppStart(testDevice: TestDevice): Promise<void> {
  expect((await testDevice.shell(`pidof ${ANDROID_PACKAGE}`)).trim()).not.toBe('');
  await expect
    .poll(async () => (await testDevice.shell('dumpsys notification')).includes(ANDROID_NOTIFICATION_CHANNEL), {
      timeout: ANDROID_CHANNEL_TIMEOUT_MS,
    })
    .toBe(true);
}

export const SUPPORTED_DEVICES: SupportedDevice[] = [
  { platform: TestDevicePlatform.Linux, channels: [ChannelKind.Network, ChannelKind.Bluetooth] },
  {
    platform: TestDevicePlatform.Android,
    channels: [ChannelKind.Network, ChannelKind.Bluetooth],
    // The two phones of `bluetooth-pair-from-zero.spec.ts`, which runs after
    // this check. The phone group starts its phones once, at the first
    // request, and cannot add a phone later, so the first request asks for two.
    appStartTestDevices: { a: TestDevicePlatform.Android, b: TestDevicePlatform.Android },
    appStart: androidAppStart,
  },
  // Only the app start for now. Add a channel when a device-support check
  // proves it on Windows.
  { platform: TestDevicePlatform.Windows, channels: [] },
];

/** True when the platform of each test device supports `channel`. */
export function supportsChannel(testDevices: TestDevice[], channel: ChannelKind): boolean {
  return testDevices.every((testDevice) => SUPPORTED_DEVICES.some(
    (device) => device.platform === testDevice.platform && device.channels.includes(channel),
  ));
}
