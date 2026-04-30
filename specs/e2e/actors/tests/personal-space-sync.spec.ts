import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';
import {
  ensurePersonalSpaceSync,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';

test('device can request Personal space sync and peer confirms it', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);

  const [labelA, labelB] = await Promise.all([
    waitForPersonalLastSyncedLabel(actorA, syncedB.identity.device_id),
    waitForPersonalLastSyncedLabel(actorB, syncedA.identity.device_id),
  ]);

  expect(labelA).toContain('last synced:');
  expect(labelB).toContain('last synced:');
});
