import { test, expect } from '../test-device.ts';

interface DeviceIdentity {
  device_id: string;
  hostname: string;
}

test('runner controls two isolated test devices with different identities', async ({ testDeviceA, testDeviceB }) => {
  await testDeviceA.page.waitForSelector('nav.nav', 30_000);
  await testDeviceB.page.waitForSelector('nav.nav', 30_000);
  await expect(await hasIncomingSyncDialog(testDeviceA.page)).toBe(false);
  await expect(await hasIncomingSyncDialog(testDeviceB.page)).toBe(false);

  const identityA = await testDeviceA.invoke<DeviceIdentity>('device_get_identity');
  const identityB = await testDeviceB.invoke<DeviceIdentity>('device_get_identity');

  expect(identityA.device_id).toBeTruthy();
  expect(identityB.device_id).toBeTruthy();
  expect(identityA.device_id).not.toBe(identityB.device_id);

  // The harness sets HOSTNAME to the slug only for a desktop process that it
  // starts. A phone has its own name, so the test asserts only that it has one.
  for (const [testDevice, identity] of [
    [testDeviceA, identityA],
    [testDeviceB, identityB],
  ] as const) {
    if (testDevice.kind === 'desktop-process') {
      expect(identity.hostname).toBe(testDevice.slug);
    } else {
      expect(identity.hostname).toBeTruthy();
    }
  }
});

async function hasIncomingSyncDialog(page: { evaluate: <T>(script: string) => Promise<T> }): Promise<boolean> {
  return page.evaluate<boolean>(`(() => Boolean(document.querySelector('[data-testid="incoming-space-sync-dialog"]')))()`);
}
