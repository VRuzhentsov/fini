import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';
import { pollUntil } from '../helpers/dom.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  openDeviceDetailsFromSettings,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';

const PERSONAL_SPACE_ID = '1';

test('device can request Personal space sync and peer confirms it', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);

  const [labelA, labelB] = await Promise.all([
    waitForPersonalLastSyncedLabel(actorA, syncedB.identity.device_id),
    waitForPersonalLastSyncedLabel(actorB, syncedA.identity.device_id),
  ]);

  expect(labelA).toContain('last synced:');
  expect(labelB).toContain('last synced:');
  await expectNoIncomingSpaceSyncDialog(actorA);
  await expectNoIncomingSpaceSyncDialog(actorB);
});

test('already mapped Personal space does not prompt again on sync tick', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);
  await actorA.invoke('space_sync_tick');
  await actorB.invoke('space_sync_tick');

  await expectNoIncomingSpaceSyncDialog(actorA);
  await expectNoIncomingSpaceSyncDialog(actorB);
});

test('ending and re-enabling Personal sync records end then bootstraps again', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);

  await actorA.invoke<string[]>('space_sync_update_mappings', {
    peerDeviceId: syncedB.identity.device_id,
    mappedSpaceIds: [],
  });
  await actorA.invoke('space_sync_tick');
  await actorB.invoke('space_sync_tick');

  // Polled, not asserted straight after the ticks above: one tick each is
  // enough for co-located spawned actors, but not for an external actor on a
  // real device -- the end-of-sync frame has an actual network hop to make.
  // Measured on a physical phone: still mapped at +0ms, cleared by +3s. The
  // assertion is unchanged, it just stops assuming the hop is instant.
  const endedStatus = await pollUntil(
    'peer records the end of Personal sync',
    async () => {
      const status = await actorB.invoke<{
        mapped_space_ids: string[];
        end_of_sync_at_by_space: Record<string, string | null>;
      }>('space_sync_status', { peerDeviceId: syncedA.identity.device_id });
      if (status.mapped_space_ids.includes(PERSONAL_SPACE_ID)) {
        return false;
      }
      return status;
    },
  );
  expect(endedStatus.mapped_space_ids).not.toContain(PERSONAL_SPACE_ID);
  expect(endedStatus.end_of_sync_at_by_space[PERSONAL_SPACE_ID]).toBeTruthy();

  await actorA.invoke<string[]>('space_sync_update_mappings', {
    peerDeviceId: syncedB.identity.device_id,
    mappedSpaceIds: [PERSONAL_SPACE_ID],
  });
  await openDeviceDetailsFromSettings(actorB, syncedA.identity.device_id);
  await actorB.page.waitForSelector(
    `[data-testid="incoming-space-sync-dialog"][data-dialog-kind="approve"][data-peer-device-id="${syncedA.identity.device_id}"][data-space-id="${PERSONAL_SPACE_ID}"]`,
    60_000,
  );
  await actorB.page.click('[data-testid="approve-space-sync"]');

  const [labelA, labelB] = await Promise.all([
    waitForPersonalLastSyncedLabel(actorA, syncedB.identity.device_id),
    waitForPersonalLastSyncedLabel(actorB, syncedA.identity.device_id),
  ]);
  expect(labelA).toContain('last synced:');
  expect(labelB).toContain('last synced:');
});
