import { expect, TestDevicePlatform } from '../test-device.ts';
import type { TestDevice } from '../test-device.ts';
import { pollUntil } from './dom.ts';
import { waitForTestDevicesStarted, type SyncedTestDevice } from './device-sync.ts';
import { openDeviceDetailsFromSettings } from './personal-sync.ts';
import { ChannelColor, ChannelKind, ChannelState } from '../../../../src/utils/channel.ts';

/**
 * The pairing for specs with the `testDeviceChannel` option
 * `ChannelKind.Bluetooth`. The harness starts virtual Linux test devices with
 * `FINI_DISCOVERY_DISABLED=1`, so they find nothing on the network, and the
 * pairing through the UI of `pairTestDevices` cannot work. The
 * Bluetooth code of the app runs as on a phone: `ble.rs` connects, accepts
 * and claims the session, on the mock radio of `ble-gatt`. The pair is saved
 * with `viaBluetooth: true`, so the Bluetooth channel is on for the pair. See
 * `fakeBluetoothAddress` in `providers/linux.ts` and
 * `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`.
 */

/**
 * The fake address of a test device comes only from its position (see
 * `fakeBluetoothAddress` in `providers/linux.ts`). This helper calculates it
 * again and does not import it: it uses only what the harness gave to each
 * test device (`FINI_LOCAL_BLUETOOTH_ADDRESS`). A test gives the address of
 * the other test device to `device_save_paired_device`.
 */
function fakeBluetoothAddress(index: number): string {
  return `AA:BB:CC:00:00:${(index + 1).toString(16).padStart(2, '0').toUpperCase()}`;
}

export async function pairTestDevicesOverBluetooth(
  testDevices: TestDevice[],
  timeoutMs = 60_000,
): Promise<SyncedTestDevice[]> {
  if (testDevices.length !== 2) {
    throw new Error(`pairTestDevicesOverBluetooth expects exactly two testDevices, got ${testDevices.length}`);
  }

  const [a, b] = await waitForTestDevicesStarted(testDevices, timeoutMs);

  // A test device with the physical source, and a virtual Android test
  // device, have a pairing made through the app. A fake address would
  // overwrite it. Thus this path only asserts the pairing, and switches on
  // the Bluetooth channel of the pair (one boolean since ADR-0006).
  if (!a.testDevice.isVirtual(TestDevicePlatform.Linux) || !b.testDevice.isVirtual(TestDevicePlatform.Linux)) {
    await switchOnBluetoothForPeer(a, b.identity.device_id);
    await switchOnBluetoothForPeer(b, a.identity.device_id);
    return [a, b];
  }

  await a.testDevice.invoke('device_save_paired_device', {
    peerDeviceId: b.identity.device_id,
    displayName: b.identity.hostname,
    bluetoothAddress: fakeBluetoothAddress(1),
    viaBluetooth: true,
    // The key that a pairing pins (ADR-0009 D8). Without it, the app
    // rejects each Bluetooth session.
    endpointId: b.identity.endpoint_id,
  });
  await b.testDevice.invoke('device_save_paired_device', {
    peerDeviceId: a.identity.device_id,
    displayName: a.identity.hostname,
    bluetoothAddress: fakeBluetoothAddress(0),
    viaBluetooth: true,
    // The key that a pairing pins (ADR-0009 D8). Without it, the app
    // rejects each Bluetooth session.
    endpointId: a.identity.endpoint_id,
  });

  return [a, b];
}

interface PairedDeviceRow {
  peer_device_id: string;
}

interface ChannelStatusRow {
  kind: ChannelKind;
  state: ChannelState;
  color: ChannelColor;
}

/**
 * Asserts the pairing that the run uses, then makes sure that its Bluetooth
 * channel is on.
 *
 * The test asserts the pairing and does not make it: the pairing on a phone
 * or a physical test device is the item that the run tests.
 *
 * The test can switch on the channel, because since ADR-0006 the switch is
 * one boolean and there is no bond of the operating system. An older build
 * sometimes switched Bluetooth off when it found no bond. Then the peer
 * rejected each session (`auth rejected: bluetooth disabled for this pair`).
 *
 * The test does not need a stored address. Since ADR-0006 the app does not
 * connect to it, and a phone that advertises with a changing address has no
 * fixed one.
 */
async function switchOnBluetoothForPeer(
  testDevice: SyncedTestDevice,
  peerDeviceId: string,
): Promise<void> {
  const paired = await testDevice.testDevice.invoke<PairedDeviceRow[]>('device_paired_devices');
  expect(
    paired.some((entry) => entry.peer_device_id === peerDeviceId),
    `${testDevice.testDevice.slug} should already be paired with ${peerDeviceId}`,
  ).toBe(true);

  // The switch is in `channels`, not on the paired row. A pair made over the
  // network has no Bluetooth row, so the paired row always says "off".
  const statuses = await testDevice.testDevice.invoke<ChannelStatusRow[]>(
    'device_channel_statuses',
    { peerDeviceId },
  );
  const bluetooth = statuses.find((status) => status.kind === ChannelKind.Bluetooth);
  if (bluetooth?.state === ChannelState.On) {
    return;
  }
  // Only the two devices together can add a channel, in the dialog on each
  // device (ADR-0008 D1). A test cannot switch on a channel that is not there.
  expect(
    bluetooth?.state,
    `${testDevice.testDevice.slug} has no Bluetooth channel with ${peerDeviceId}: add it in the app first`,
  ).toBe(ChannelState.Off);

  const after = await testDevice.testDevice.invoke<ChannelStatusRow[]>(
    'device_set_channel_enabled',
    { peerDeviceId, kind: ChannelKind.Bluetooth, enabled: true },
  );
  expect(
    after.find((status) => status.kind === ChannelKind.Bluetooth)?.state,
    `${testDevice.testDevice.slug} should have Bluetooth on for ${peerDeviceId}`,
  ).toBe(ChannelState.On);
}

/**
 * Proves that the Network channel cannot carry the session, so that a
 * Bluetooth result does not come from the network.
 *
 * There are two checks. The harness starts a virtual Linux test device with
 * `FINI_DISCOVERY_DISABLED=1`, so it hears no device: the test asserts that.
 * Each other test device can hear its peer over Wi-Fi. There, the test
 * asserts that the Network channel of the pair is not on.
 */
export async function expectNetworkChannelUnavailable(
  testDevice: TestDevice,
  peerDeviceId?: string,
): Promise<void> {
  // A virtual Linux test device started with FINI_DISCOVERY_DISABLED=1 and
  // hears no device. This check is stronger, and it also proves the flag.
  if (testDevice.isVirtual(TestDevicePlatform.Linux) || peerDeviceId === undefined) {
    const presence = await testDevice.invoke<unknown[]>('device_heard_devices');
    expect(presence, `${testDevice.slug} should have no network presence (FINI_DISCOVERY_DISABLED)`).toHaveLength(0);
    return;
  }

  const statuses = await testDevice.invoke<ChannelStatusRow[]>('device_channel_statuses', { peerDeviceId });
  const network = statuses.find((status) => status.kind === ChannelKind.Network);
  expect(
    network?.state,
    `${testDevice.slug} has the Network channel on with ${peerDeviceId}: ` +
      'this test needs a pair that has only the Bluetooth channel',
  ).not.toBe(ChannelState.On);
}

/**
 * The Rust side of the app sets green (ADR-0008 D9): the app heard the peer
 * on this channel in its time limit, or an exchange with it is open.
 */
export async function waitForGreenChannel(
  testDevice: TestDevice,
  peerDeviceId: string,
  timeoutMs = 60_000,
): Promise<void> {
  await pollUntil(`${testDevice.slug} Bluetooth channel is green`, async () => {
    await testDevice.invoke('space_sync_tick');
    const statuses = await testDevice.invoke<ChannelStatusRow[]>('device_channel_statuses', {
      peerDeviceId,
    });
    const bluetooth = statuses.find((status) => status.kind === ChannelKind.Bluetooth);
    return bluetooth?.color === ChannelColor.Green || false;
  }, timeoutMs, 1_000);
}

/**
 * Green on the screen: the Bluetooth row of the Device page shows the color
 * that the Rust side sets. The test reads `data-channel-color`, not the text
 * of the row, because the text changes with the design.
 *
 * Orange stops the test at once: it is a problem of the radio on this
 * device, and more time does not change it.
 */
export async function waitForBluetoothRowConnectedInUi(
  testDevice: TestDevice,
  peerDeviceId: string,
  timeoutMs = 60_000,
): Promise<void> {
  const selector = `[data-testid="channel-status-row"][data-channel-kind="${ChannelKind.Bluetooth}"]`;

  // The test opens the page one time and keeps it open. The open page runs
  // the status search (ADR-0008 D12) and reads its rows again each few
  // seconds. When the test opened it at each check, the search started again
  // each time, and it did not run for sufficient time to hear the peer.
  await openDeviceDetailsFromSettings(testDevice, peerDeviceId);
  await pollUntil(`${testDevice.slug} bluetooth row is green in the UI`, async () => {
    await testDevice.invoke('space_sync_tick');

    const row = testDevice.page.locator(selector);
    const color = (await row.count()) > 0 ? ((await row.getAttribute('data-channel-color')) ?? '') : '';
    if (color === '') {
      await openDeviceDetailsFromSettings(testDevice, peerDeviceId);
      return false;
    }

    if (color === ChannelColor.Orange) {
      throw new Error(`${testDevice.slug} bluetooth row reports a problem on this device`);
    }

    return color === ChannelColor.Green ? color : false;
  }, timeoutMs, 1_000);
}
