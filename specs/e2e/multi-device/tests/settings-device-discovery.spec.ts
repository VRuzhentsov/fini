import { test } from '../test-device.ts';
import { ensureSyncedTestDevices } from '../helpers/device-sync.ts';

test('two app instances pair through Settings and show each other device names', async ({ testDeviceA, testDeviceB }) => {
  await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
});
