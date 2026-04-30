import { test } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';

test('two app instances pair through Settings and show each other device names', async ({ actorA, actorB }) => {
  await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
});
