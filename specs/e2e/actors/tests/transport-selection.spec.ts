import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';

/**
 * Companion to `peer-sync-over-sim.spec.ts`: proves the network-first half
 * of primary-transport selection in the real app. Normal actors (network
 * transport available, the default/common case) must report `tcp_ws` as
 * primary via the same `device_connection_session_transport` surface the
 * Sim test asserts `sim` on. Together the two specs prove primary
 * selection end-to-end: network wins whenever it's connected. See
 * `specs/e2e/transports.md`.
 */
test('paired actors with network available claim their session as the network transport', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  const kindOnA = await actorA.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedB.identity.device_id,
  });
  const kindOnB = await actorB.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedA.identity.device_id,
  });

  expect(kindOnA).toBe('tcp_ws');
  expect(kindOnB).toBe('tcp_ws');
});
