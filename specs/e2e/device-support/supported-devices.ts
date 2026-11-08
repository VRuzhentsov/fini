// The platforms that Fini supports, and the channels on each (ADR-0013 D11).
// Only a list: it picks no test devices. Each device-support spec names its
// own test devices with `test.use`.

import { ChannelKind } from '../../../src/utils/channel.ts';
import type { TestDevice } from '../multi-device/test-device.ts';
import { TestDevicePlatform } from '../multi-device/test-device-platform.ts';

export interface SupportedDevice {
  platform: TestDevicePlatform;
  channels: ChannelKind[];
}

export const SUPPORTED_DEVICES: SupportedDevice[] = [
  { platform: TestDevicePlatform.Linux, channels: [ChannelKind.Network, ChannelKind.Bluetooth] },
  { platform: TestDevicePlatform.Android, channels: [ChannelKind.Network, ChannelKind.Bluetooth] },
  // Only the app start for now (`windows-app-start.spec.ts`). Add a channel
  // when a device-support check proves it on Windows.
  { platform: TestDevicePlatform.Windows, channels: [] },
];

/** True when the platform of each test device supports `channel`. */
export function supportsChannel(testDevices: TestDevice[], channel: ChannelKind): boolean {
  return testDevices.every((testDevice) => SUPPORTED_DEVICES.some(
    (device) => device.platform === testDevice.platform && device.channels.includes(channel),
  ));
}
