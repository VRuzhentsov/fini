import { test, expect } from '../test-device.ts';
import { pairTestDevices } from '../helpers/device-sync.ts';
import {
  syncPersonalSpace,
  openDeviceDetailsFromSettings,
  waitForMappingControls,
} from '../helpers/personal-sync.ts';
import {
  anyChannelConnected,
  deviceDotConnected,
  switchChannel,
  waitForChannelColor,
  waitForSyncQueue,
} from '../helpers/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * The Device page of ADR-0007: each channel has its switch, there is a sync
 * queue, and the unpair dialog tells what stops.
 *
 * Unit tests cannot prove this. `DeviceView.spec.ts` replaces the store, so
 * it proves only that the page shows what it gets. These tests prove it on
 * two running apps: a switch must stop the data.
 */

test('a channel switched off stops the data, and not only the color of the row', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await pairTestDevices([testDeviceA, testDeviceB], { pairInUi: true });
  await syncPersonalSpace(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
  );

  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green);

  // Off. A grey row with an open session would show an incorrect state, so
  // the test asserts that the session is closed, not only the row.
  await switchChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Off);

  // The test checks 10 times, not one time. One check immediately after the
  // switch passes also when the switch does nothing, because the peer has
  // not connected again yet. On hardware, this check passed one time, and
  // the session opened again some seconds later. The switch must stop the
  // connections from this device and reject the connections from the peer
  // (`check_network_enabled`).
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const channel = await testDeviceA.invoke<string | null>('space_sync_session_channel', {
      peerDeviceId: syncedB.identity.device_id,
    });
    expect(
      channel,
      'the Network session stays closed while the channel is off, also when the peer connects',
    ).not.toBe(ChannelKind.Network);
    await testDeviceA.invoke('space_sync_tick');
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }

  // On again: the switch must also switch the channel on.
  await switchChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green, 60_000);
});

/**
 * The app cannot switch on a Bluetooth channel when the radio of the device
 * does not operate (ADR-0008 D6). The e2e container has a D-Bus system bus
 * with no `bluetoothd` (see `scripts/e2e-runner.sh`), so there is no adapter:
 * the same condition as Bluetooth off on a laptop.
 *
 * The page must say that the problem is on this device, and the app writes
 * nothing: the switch stays off (ADR-0011).
 */
test('the app rejects Bluetooth on with no radio, and names this device', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [, syncedB] = await pairTestDevices([testDeviceA, testDeviceB], { pairInUi: true });

  await openDeviceDetailsFromSettings(testDeviceA, syncedB.identity.device_id);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Bluetooth, ChannelColor.Off);

  await switchChannel(testDeviceA, ChannelKind.Bluetooth);
  await testDeviceA.page.waitForSelector('[data-testid="channel-error"]', 30_000);
  const failure = ((await testDeviceA.page.getByTestId('channel-error').textContent()) ?? '').trim();
  expect(failure, 'the error names this device').toContain('this device');

  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Bluetooth, ChannelColor.Off);
});

/**
 * The sync queue tells the person if their data is synced.
 *
 * The test first switches off the channel. Then no data can go to the peer,
 * and the queue must show the waiting changes; a sync tick cannot empty it
 * first. When the channel is on again, the queue becomes empty: it does not
 * only show a number.
 */
test('the sync queue shows the changes that wait, then that they are synced', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await pairTestDevices([testDeviceA, testDeviceB], { pairInUi: true });
  await syncPersonalSpace(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
  );

  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green);
  await switchChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Off);

  const title = `queued quest ${Date.now()}`;
  await testDeviceA.invoke('create_quest', {
    input: { title, description: null, space_id: '1', is_checklist: false },
  });

  const stalled = await waitForSyncQueue(
    testDeviceA,
    syncedB.identity.device_id,
    (text) => text.includes('waiting'),
    'shows changes that wait while no data can go to the peer',
  );
  expect(stalled).toContain('waiting');

  await switchChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, syncedB.identity.device_id, ChannelKind.Network, ChannelColor.Green, 60_000);

  const drained = await waitForSyncQueue(
    testDeviceA,
    syncedB.identity.device_id,
    (text) => text.includes('Everything synced'),
    'becomes empty when the channel is on again',
    60_000,
  );
  expect(drained).toContain('Everything synced');
});

/**
 * The unpair dialog names the spaces that stop their sync, and says that the
 * app deletes nothing. Only "Are you sure?" does not help the person to
 * decide.
 */
test('the unpair dialog names the spaces that stop their sync, and says that nothing is deleted', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await pairTestDevices([testDeviceA, testDeviceB], { pairInUi: true });
  await syncPersonalSpace(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
  );

  await openDeviceDetailsFromSettings(testDeviceA, syncedB.identity.device_id);
  // The page shows its space rows from the space list, which comes before
  // the mappings of the pair. Without this wait, the test read the names too
  // early, and usually found none.
  await waitForMappingControls(testDeviceA, '1');
  await testDeviceA.page.waitForSelector('[data-testid="unlink-device"]', 30_000);
  await testDeviceA.page.click('[data-testid="unlink-device"]');

  const confirmation = ((await testDeviceA.page.getByTestId('unlink-dialog').textContent()) ?? '')
    .replace(/\s+/g, ' ')
    .trim();

  expect(confirmation).toContain('Personal');
  expect(confirmation).toContain('Nothing is deleted');
});

/**
 * The dot in the devices list, and no line of text under it.
 *
 * The dot showed only that the app heard a beacon on the LAN. Thus a device
 * that the app only heard was green with no session. A person found this
 * defect, not a test. This test compares the dot with the channel rows of
 * the Device page: the two show one fact, and the defect was that they did
 * not agree. Two fixed values would let them become different again.
 */
test('the devices list dot agrees with the channel rows', async ({ testDeviceA, testDeviceB }) => {
  const [, syncedB] = await pairTestDevices([testDeviceA, testDeviceB], { pairInUi: true });
  const peerId = syncedB.identity.device_id;

  await waitForChannelColor(testDeviceA, peerId, ChannelKind.Network, ChannelColor.Green);
  expect(await anyChannelConnected(testDeviceA, peerId), 'a channel row is connected').toBe(true);
  expect(await deviceDotConnected(testDeviceA, peerId), 'so the dot is the connected colour').toBe(true);

  // No channel is connected. The dot shows the channels: when the only
  // channel is off, nothing is green, also when the app hears the beacon.
  await openDeviceDetailsFromSettings(testDeviceA, peerId);
  await switchChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, peerId, ChannelKind.Network, ChannelColor.Off);

  expect(await anyChannelConnected(testDeviceA, peerId), 'no channel row is connected').toBe(false);
  expect(await deviceDotConnected(testDeviceA, peerId), 'so the dot is not green').toBe(false);

  // The row shows no text. The person switched off the channel; it is not a
  // problem on this device, so there is no ⓘ: it explains only problems
  // (ADR-0008 D19).
  const row = `[data-testid="paired-device-row"][data-peer-device-id="${peerId}"]`;
  const detail = {
    shown: (await testDeviceA.page.locator(`${row} [data-testid="paired-device-detail"]`).count()) > 0,
    hasInfo: (await testDeviceA.page.locator(`${row} [data-testid="paired-device-info"]`).count()) > 0,
  };
  expect(detail.shown, 'the list shows no text for each device').toBe(false);
  expect(detail.hasInfo, 'a channel switched off is not a problem to explain').toBe(false);

  await openDeviceDetailsFromSettings(testDeviceA, peerId);
  await switchChannel(testDeviceA, ChannelKind.Network);
  await waitForChannelColor(testDeviceA, peerId, ChannelKind.Network, ChannelColor.Green, 60_000);
});
