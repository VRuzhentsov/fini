import { test, expect } from '../test-device.ts';
import { ensureSyncedTestDevices } from '../helpers/device-sync.ts';
import { pollUntil } from '../helpers/dom.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  openDeviceDetailsFromSettings,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';

const PERSONAL_SPACE_ID = '1';

test('device can request Personal space sync and peer confirms it', async ({ testDeviceA, testDeviceB }) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });

  await ensurePersonalSpaceSync(testDeviceA, syncedB.identity.device_id, testDeviceB, syncedA.identity.device_id);

  const [labelA, labelB] = await Promise.all([
    waitForPersonalLastSyncedLabel(testDeviceA, syncedB.identity.device_id),
    waitForPersonalLastSyncedLabel(testDeviceB, syncedA.identity.device_id),
  ]);

  expect(labelA).toContain('last synced:');
  expect(labelB).toContain('last synced:');
  await expectNoIncomingSpaceSyncDialog(testDeviceA);
  await expectNoIncomingSpaceSyncDialog(testDeviceB);
});

test('already mapped Personal space does not prompt again on sync tick', async ({ testDeviceA, testDeviceB }) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });

  await ensurePersonalSpaceSync(testDeviceA, syncedB.identity.device_id, testDeviceB, syncedA.identity.device_id);
  await testDeviceA.invoke('space_sync_tick');
  await testDeviceB.invoke('space_sync_tick');

  await expectNoIncomingSpaceSyncDialog(testDeviceA);
  await expectNoIncomingSpaceSyncDialog(testDeviceB);
});

test('ending and re-enabling Personal sync records end then bootstraps again', async ({ testDeviceA, testDeviceB }) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });

  await ensurePersonalSpaceSync(testDeviceA, syncedB.identity.device_id, testDeviceB, syncedA.identity.device_id);

  await testDeviceA.invoke<string[]>('space_sync_update_mappings', {
    peerDeviceId: syncedB.identity.device_id,
    mappedSpaceIds: [],
  });
  await testDeviceA.invoke('space_sync_tick');
  await testDeviceB.invoke('space_sync_tick');

  // Polled, not asserted straight after the ticks above: one tick each is
  // enough for co-located spawned testDevices, but not for an external testDevice on a
  // real device -- the end-of-sync frame has an actual network hop to make.
  // Measured on a physical phone: still mapped at +0ms, cleared by +3s. The
  // assertion is unchanged, it just stops assuming the hop is instant.
  const endedStatus = await pollUntil(
    'peer records the end of Personal sync',
    async () => {
      const status = await testDeviceB.invoke<{
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

  await testDeviceA.invoke<string[]>('space_sync_update_mappings', {
    peerDeviceId: syncedB.identity.device_id,
    mappedSpaceIds: [PERSONAL_SPACE_ID],
  });
  await openDeviceDetailsFromSettings(testDeviceB, syncedA.identity.device_id);
  await testDeviceB.page.waitForSelector(
    `[data-testid="incoming-space-sync-dialog"][data-dialog-kind="approve"][data-peer-device-id="${syncedA.identity.device_id}"][data-space-id="${PERSONAL_SPACE_ID}"]`,
    60_000,
  );
  await testDeviceB.page.click('[data-testid="approve-space-sync"]');

  const [labelA, labelB] = await Promise.all([
    waitForPersonalLastSyncedLabel(testDeviceA, syncedB.identity.device_id),
    waitForPersonalLastSyncedLabel(testDeviceB, syncedA.identity.device_id),
  ]);
  expect(labelA).toContain('last synced:');
  expect(labelB).toContain('last synced:');
});
