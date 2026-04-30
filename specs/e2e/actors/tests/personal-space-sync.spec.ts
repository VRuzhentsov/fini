import { test, expect } from '../fixtures.js';
import { ensureSyncedActors } from '../helpers/device-sync.js';
import {
  ensurePersonalSpaceSync,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.js';

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
