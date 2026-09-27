import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';
import { ensurePersonalSpaceSync } from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';
import { ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * Companion to `peer-sync-over-ble.spec.ts`: proves the network-first half
 * of channel selection in the real app. Two actors on one network have work
 * to move, so an exchange opens (ADR-0008 D10), and it must run over the
 * Network channel. Together the two specs prove selection end-to-end:
 * network wins whenever the peer is present on it. See
 * `specs/e2e/transports.md`.
 */
test('an exchange between paired actors on one network runs over the network channel', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  // An idle pair holds no exchange; saving a mapping is work, and the
  // exchange that carries it stays open until it has been idle for a while.
  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);
  await actorA.invoke('create_quest', {
    input: { title: `network exchange ${Date.now()}`, description: null, space_id: '1', is_checklist: false },
  });

  const kindOnA = await pollUntil('actor-a has an exchange open', async () => {
    await actorA.invoke('space_sync_tick');
    const kind = await actorA.invoke<string | null>('device_connection_session_channel', {
      peerDeviceId: syncedB.identity.device_id,
    });
    return kind ?? false;
  }, 30_000, 250);

  expect(kindOnA).toBe(ChannelKind.Network);
});
