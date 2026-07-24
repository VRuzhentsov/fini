import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';

/**
 * Companion to `peer-sync-over-sim.spec.ts`: proves the network-first half
 * of transport selection in the real app. Normal actors (network transport
 * available, the default/common case) must claim their session as `tcp_ws`
 * — never falling back — via the same `device_connection_session_transport`
 * surface the Sim-fallback test asserts `sim` on. Together the two specs
 * prove selection end-to-end: network preferred when available, fallback
 * only when it genuinely is not. See `specs/e2e/transports.md`.
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
