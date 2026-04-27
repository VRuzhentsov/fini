import { test, expect } from '../fixtures.js';

interface DeviceIdentity {
  device_id: string;
  hostname: string;
}

test('runner controls two isolated actors with distinct identities', async ({ actorA, actorB }) => {
  await actorA.page.waitForSelector('nav.nav', 30_000);
  await actorB.page.waitForSelector('nav.nav', 30_000);

  const identityA = await actorA.invoke<DeviceIdentity>('device_connection_get_identity');
  const identityB = await actorB.invoke<DeviceIdentity>('device_connection_get_identity');

  expect(identityA.device_id).toBeTruthy();
  expect(identityB.device_id).toBeTruthy();
  expect(identityA.device_id).not.toBe(identityB.device_id);
  expect(identityA.hostname).toBe('actor-a');
  expect(identityB.hostname).toBe('actor-b');
});
