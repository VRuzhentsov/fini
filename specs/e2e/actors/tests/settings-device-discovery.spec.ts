import { test } from '../fixtures.js';
import { ensureSyncedActors } from '../helpers/device-sync.js';

test('two app instances pair through Settings and show each other device names', async ({ actorA, actorB }) => {
  await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
});
