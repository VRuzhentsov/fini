import { test, expect } from '../test-device.ts';
import { ensureSyncedTestDevices } from '../helpers/device-sync.ts';
import { ensurePersonalSpaceSync } from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';
import { ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * Proves the first part of the channel order in the app; the other part is
 * in `peer-sync-over-ble.spec.ts`. Two test devices on one network have work
 * to send, so an exchange opens (ADR-0008 D10). It must use the Network
 * channel. The two specs together prove the channel order: the app uses
 * Network when it hears the peer there. See `docs/e2e.md`, "What each
 * project proves for each channel".
 */
test('an exchange between paired test devices on one network runs over the network channel', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });

  // A pair with no work has no exchange. A saved mapping is work, and its
  // exchange stays open for some time after the work stops.
  await ensurePersonalSpaceSync(testDeviceA, syncedB.identity.device_id, testDeviceB, syncedA.identity.device_id);
  await testDeviceA.invoke('create_quest', {
    input: { title: `network exchange ${Date.now()}`, description: null, space_id: '1', is_checklist: false },
  });

  const kindOnA = await pollUntil(`${testDeviceA.slug} has an exchange open`, async () => {
    await testDeviceA.invoke('space_sync_tick');
    const kind = await testDeviceA.invoke<string | null>('space_sync_session_channel', {
      peerDeviceId: syncedB.identity.device_id,
    });
    return kind ?? false;
  }, 30_000, 250);

  expect(kindOnA).toBe(ChannelKind.Network);
});
