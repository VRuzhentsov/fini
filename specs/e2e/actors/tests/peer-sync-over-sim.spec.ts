import { test, expect } from '../fixtures.ts';
import {
  ensureSimPairedActors,
  expectNetworkTransportUnavailable,
  waitForSimSession,
} from '../helpers/sim-sync.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';

/**
 * Proves the transport abstraction with a real second transport, not a
 * mock: two real `fini-app` processes, network transport made genuinely
 * unavailable (`FINI_E2E_TRANSPORT=sim` -> `FINI_DISCOVERY_DISABLED=1`),
 * establish an authenticated session over the Sim adapter and replicate an
 * approved Space over it — the same acceptance shape the ticket requires
 * for the real Bluetooth fallback. See `specs/e2e/transports.md`.
 */
test('peer session establishes over Sim transport when network is unavailable, and Space sync replicates over it', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureSimPairedActors([actorA, actorB]);

  await expectNetworkTransportUnavailable(actorA);
  await expectNetworkTransportUnavailable(actorB);

  await waitForSimSession(actorA);
  await waitForSimSession(actorB);

  const kindOnA = await actorA.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedB.identity.device_id,
  });
  const kindOnB = await actorB.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedA.identity.device_id,
  });
  expect(kindOnA).toBe('sim');
  expect(kindOnB).toBe('sim');

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);

  const [labelA, labelB] = await Promise.all([
    waitForPersonalLastSyncedLabel(actorA, syncedB.identity.device_id),
    waitForPersonalLastSyncedLabel(actorB, syncedA.identity.device_id),
  ]);

  expect(labelA).toContain('last synced:');
  expect(labelB).toContain('last synced:');
  await expectNoIncomingSpaceSyncDialog(actorA);
  await expectNoIncomingSpaceSyncDialog(actorB);

  // Session claim is sticky: still the same transport after further ticks,
  // never re-negotiated mid-session.
  await actorA.invoke('space_sync_tick');
  const kindOnAAfterTick = await actorA.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedB.identity.device_id,
  });
  expect(kindOnAAfterTick).toBe('sim');
});
