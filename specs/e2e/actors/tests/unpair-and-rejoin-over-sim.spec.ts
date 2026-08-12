import { test, expect } from '../fixtures.ts';
import { ensureSimPairedActors, waitForSimSession } from '../helpers/sim-sync.ts';
import { ensurePersonalSpaceSync, waitForPersonalLastSyncedLabel } from '../helpers/personal-sync.ts';

interface PairedDeviceRow {
  peer_device_id: string;
}

/**
 * The "delete Bluetooth connection, then pair again" lifecycle: unpairing
 * fully deletes the paired_devices row (cascading to pair_space_mappings,
 * see migration 00000000000010's ON DELETE CASCADE), so a subsequent
 * pairing goes through the exact same fresh-insert path a first-ever pair
 * does -- not a resumed/patched-up one. This proves that full cycle
 * recovers completely: session re-establishes, and Space sync has to be
 * re-requested and re-approved from scratch (the mapping is genuinely
 * gone, not just hidden), not silently inherited from before the unpair.
 */
test('unpairing and re-pairing over Sim transport fully recovers session and Space sync', async ({
  actorA,
  actorB,
}) => {
  const [firstSyncedA, firstSyncedB] = await ensureSimPairedActors([actorA, actorB]);
  await waitForSimSession(actorA);
  await waitForSimSession(actorB);
  await ensurePersonalSpaceSync(
    actorA,
    firstSyncedB.identity.device_id,
    actorB,
    firstSyncedA.identity.device_id,
  );
  const labelBeforeUnpair = await waitForPersonalLastSyncedLabel(actorA, firstSyncedB.identity.device_id);
  expect(labelBeforeUnpair).toContain('last synced:');

  await actorA.invoke('device_connection_unpair', { peerDeviceId: firstSyncedB.identity.device_id });
  await actorB.invoke('device_connection_unpair', { peerDeviceId: firstSyncedA.identity.device_id });

  const pairedOnAAfterUnpair = await actorA.invoke<PairedDeviceRow[]>('device_connection_get_paired_devices');
  const pairedOnBAfterUnpair = await actorB.invoke<PairedDeviceRow[]>('device_connection_get_paired_devices');
  expect(pairedOnAAfterUnpair.some((row) => row.peer_device_id === firstSyncedB.identity.device_id)).toBe(false);
  expect(pairedOnBAfterUnpair.some((row) => row.peer_device_id === firstSyncedA.identity.device_id)).toBe(false);

  const [syncedA, syncedB] = await ensureSimPairedActors([actorA, actorB]);
  await waitForSimSession(actorA);
  await waitForSimSession(actorB);

  const mappedOnAAfterRepair = await actorA.invoke<string[]>('space_sync_list_mappings', {
    peerDeviceId: syncedB.identity.device_id,
  });
  expect(
    mappedOnAAfterRepair,
    'unpair must cascade-delete the old Space mapping, not carry it over into the fresh pair',
  ).not.toContain('1');

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);
  const labelAfterRepair = await waitForPersonalLastSyncedLabel(actorA, syncedB.identity.device_id);
  expect(labelAfterRepair).toContain('last synced:');
});
