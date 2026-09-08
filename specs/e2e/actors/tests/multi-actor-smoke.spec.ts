import { test, expect } from '../fixtures.ts';

interface DeviceIdentity {
  device_id: string;
  hostname: string;
}

test('runner controls two isolated actors with distinct identities', async ({ actorA, actorB }) => {
  await actorA.page.waitForSelector('nav.nav', 30_000);
  await actorB.page.waitForSelector('nav.nav', 30_000);
  await expect(await hasIncomingSyncDialog(actorA.page)).toBe(false);
  await expect(await hasIncomingSyncDialog(actorB.page)).toBe(false);

  const identityA = await actorA.invoke<DeviceIdentity>('device_connection_get_identity');
  const identityB = await actorB.invoke<DeviceIdentity>('device_connection_get_identity');

  expect(identityA.device_id).toBeTruthy();
  expect(identityB.device_id).toBeTruthy();
  expect(identityA.device_id).not.toBe(identityB.device_id);

  // Hostname is only the harness's to assert for actors it spawned -- it
  // sets HOSTNAME to the slug there. An external actor is a real machine or
  // device that already has its own name, so pinning one would make this
  // test about the operator's environment rather than about the app.
  for (const [actor, identity] of [
    [actorA, identityA],
    [actorB, identityB],
  ] as const) {
    if (actor.kind === 'spawned') {
      expect(identity.hostname).toBe(actor.slug);
    } else {
      expect(identity.hostname).toBeTruthy();
    }
  }
});

async function hasIncomingSyncDialog(page: { evaluate: <T>(script: string) => Promise<T> }): Promise<boolean> {
  return page.evaluate<boolean>(`(() => Boolean(document.querySelector('[data-testid="incoming-space-sync-dialog"]')))()`);
}
