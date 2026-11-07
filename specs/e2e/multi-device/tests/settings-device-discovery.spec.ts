import { test } from '../test-device.ts';
import { pairTestDevices } from '../helpers/device-sync.ts';

test('two app instances pair through Settings and show each other device names', async ({ testDeviceA, testDeviceB }) => {
  await pairTestDevices([testDeviceA, testDeviceB], { pairInUi: true });
});
