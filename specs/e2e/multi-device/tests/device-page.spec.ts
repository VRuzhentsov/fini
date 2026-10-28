import { test, expect } from '../test-device.ts';
import { ensureSyncedTestDevices } from '../helpers/device-sync.ts';
import {
  ensurePersonalSpaceSync,
  openDeviceDetailsFromSettings,
  waitForMappingControlsReady,
} from '../helpers/personal-sync.ts';
import {
  anyChannelConnected,
  deviceDotConnected,
  toggleChannel,
  waitForChannelColor,
  waitForSyncQueue,
} from '../helpers/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * The Device page, as ADR-0007 rebuilt it: channels with their own
 * switches, a sync queue, and an unlink that says what it costs.
 *
 * These cover what unit tests structurally cannot. `DeviceView.spec.ts`
 * mocks the store, so it proves the page renders what it is handed --
 * these prove two real apps actually behave that way, which is the part
 * that matters for a switch whose whole job is to stop traffic.
 */

test('turning a channel off stops the traffic, not just the colour of the row', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
  await ensurePersonalSpaceSync(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
  );

  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green);

  // Off. A greyed row over a live session would be the exact lie this
  // design exists to remove, so assert the session itself is gone rather
  // than trusting the row.
  await toggleChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Off);

  // Sampled repeatedly, not once. A single check moments after the toggle
  // passes even when the switch does nothing: the peer's own dial loop simply
  // has not come back round yet. That is exactly how this assertion passed
  // while the session was in fact being re-established seconds later --
  // caught on hardware, where the peer dials on its own schedule rather than
  // the test's. Closing our dial loop is only half the switch; refusing the
  // peer's inbound dial is the other half (`check_network_enabled`).
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const transport = await testDeviceA.invoke<string | null>('space_sync_session_channel', {
      peerDeviceId: syncedB.identity.device_id,
    });
    expect(
      transport,
      'network session must stay closed while the channel is off, including against an inbound dial',
    ).not.toBe(ChannelKind.Network);
    await testDeviceA.invoke('space_sync_tick');
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }

  // And back on again: the switch has to be a switch, not a one-way door.
  await toggleChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green, 60_000);
});

/**
 * A Bluetooth channel cannot be switched on on a device whose radio is unusable
 * (ADR-0008 D6). The e2e container runs a D-Bus system bus with no
 * `bluetoothd` on it (see `scripts/e2e-runner.sh`), so the adapter genuinely
 * is not there -- the same condition as Bluetooth switched off on a laptop,
 * arrived at honestly rather than mocked.
 *
 * The page must say so about *this* device, and nothing may be written:
 * the switch stays off (ADR-0011).
 */
test('switching Bluetooth on with no usable radio is refused, and blames this device', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });

  await openDeviceDetailsFromSettings(testDeviceA, syncedB.identity.device_id);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Bluetooth, ChannelColor.Off);

  await toggleChannel(testDeviceA, ChannelKind.Bluetooth);
  await testDeviceA.page.waitForSelector('[data-testid="channel-error"]', 30_000);
  const failure = ((await testDeviceA.page.getByTestId('channel-error').textContent()) ?? '').trim();
  expect(failure, 'the failure must name this device').toContain('this device');

  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Bluetooth, ChannelColor.Off);
});

/**
 * The sync queue answers "has my stuff synced?".
 *
 * Made deterministic by closing the channel first: with nothing able to
 * reach the peer, the queue has to report a backlog rather than racing a
 * tick that would drain it. Turning the channel back on then proves the
 * queue actually drains rather than merely displaying a number.
 */
test('the sync queue reports what cannot move, then that it has moved', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
  await ensurePersonalSpaceSync(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
  );

  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green);
  await toggleChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Off);

  const title = `queued quest ${Date.now()}`;
  await testDeviceA.invoke('create_quest', {
    input: { title, description: null, space_id: '1', is_checklist: false },
  });

  const stalled = await waitForSyncQueue(
    testDeviceA,
    syncedB.identity.device_id,
    (text) => text.includes('waiting'),
    'reports a backlog while nothing can reach the peer',
  );
  expect(stalled).toContain('waiting');

  await toggleChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green, 60_000);

  const drained = await waitForSyncQueue(
    testDeviceA,
    syncedB.identity.device_id,
    (text) => text.includes('Everything synced'),
    'drains once the channel is back',
    60_000,
  );
  expect(drained).toContain('Everything synced');
});

/**
 * Unlinking states what actually stops -- the spaces, by name -- and that
 * nothing is deleted. A generic "are you sure?" gives the person nothing
 * to decide on.
 */
test('unlinking names the spaces that stop syncing and promises nothing is deleted', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
  await ensurePersonalSpaceSync(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
  );

  await openDeviceDetailsFromSettings(testDeviceA, syncedB.identity.device_id);
  // The page renders its space rows from the space list, which arrives
  // before the per-pair mappings do. Asserting on the names without waiting
  // for those read whatever had loaded so far -- usually nothing.
  await waitForMappingControlsReady(testDeviceA, '1');
  await testDeviceA.page.waitForSelector('[data-testid="unlink-device"]', 30_000);
  await testDeviceA.page.click('[data-testid="unlink-device"]');

  const confirmation = ((await testDeviceA.page.getByTestId('unlink-dialog').textContent()) ?? '')
    .replace(/\s+/g, ' ')
    .trim();

  expect(confirmation).toContain('Personal');
  expect(confirmation).toContain('Nothing is deleted');
});

/**
 * The devices list's dot, and the line that is no longer under it.
 *
 * The dot used to read presence -- a beacon heard on the LAN -- so a machine
 * that was merely discoverable showed green while no session existed. Found
 * by using the app, not by a test, which is why this one asserts the two
 * surfaces against *each other* rather than hard-coding both: the list's dot
 * and the device page's rows are two renderings of one fact, and the bug was
 * them disagreeing. Hard-coding each separately would let them drift apart
 * again and still pass.
 */
test('the devices list dot agrees with the channel rows', async ({ testDeviceA, testDeviceB }) => {
  const [, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
  const peerId = syncedB.identity.device_id;

  await waitForChannelColor(testDeviceA, peerId, ChannelKind.Network, ChannelColor.Green);
  expect(await anyChannelConnected(testDeviceA, peerId), 'a channel row is connected').toBe(true);
  expect(await deviceDotConnected(testDeviceA, peerId), 'so the dot is the connected colour').toBe(true);

  // Nothing connected. The dot follows the channels: with the only channel
  // switched off, nothing is green, whatever the network beacon says.
  await openDeviceDetailsFromSettings(testDeviceA, peerId);
  await toggleChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, peerId, ChannelKind.Network, ChannelColor.Off);

  expect(await anyChannelConnected(testDeviceA, peerId), 'no channel row is connected').toBe(false);
  expect(await deviceDotConnected(testDeviceA, peerId), 'so the dot must not be green').toBe(false);

  // And the row says nothing on its own. A channel switched off is the
  // person's own choice, not a problem on this device, so there is no ⓘ
  // either: it explains problems only (ADR-0008 D19).
  const row = `[data-testid="paired-device-row"][data-peer-device-id="${peerId}"]`;
  const detail = {
    shown: (await testDeviceA.page.locator(`${row} [data-testid="paired-device-detail"]`).count()) > 0,
    hasInfo: (await testDeviceA.page.locator(`${row} [data-testid="paired-device-info"]`).count()) > 0,
  };
  expect(detail.shown, 'the list must not explain each device unasked').toBe(false);
  expect(detail.hasInfo, 'a channel switched off is not a problem to explain').toBe(false);

  await openDeviceDetailsFromSettings(testDeviceA, peerId);
  await toggleChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, peerId, ChannelKind.Network, ChannelColor.Green, 60_000);
});
