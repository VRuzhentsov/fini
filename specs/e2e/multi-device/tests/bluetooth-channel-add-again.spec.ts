import { expect, test } from '../test-device.ts';
import { DevicePage } from '../../components/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * Bluetooth switched off and on again on a pair (ADR-0011): a channel is a
 * switch on each device. The second test device (testDeviceB) switches
 * Bluetooth off; it rejects the session, and the row of the first test device
 * (testDeviceA), still on, turns grey. testDeviceB switches it on again, and
 * the row turns green on both.
 *
 * The 10 s standard (`docs/architecture/communication.md`) is asserted from
 * the moment testDeviceB switches on until each row is green. The times of
 * the other steps are recorded as annotations.
 *
 * Starts from a pair that has Bluetooth on (run `bluetooth-pair-from-zero`
 * first, or pair by hand). Needs two radios; run as in that spec.
 */

const DISCOVERY_STANDARD_MS = 10_000;
const STEP_TIMEOUT_MS = 120_000;
/** How long the first test device can take to see the session end. */
const FOLLOW_TIMEOUT_MS = 60_000;
interface Identity {
  device_id: string;
}

function note(step: string, startedAt: number): number {
  const ms = Date.now() - startedAt;
  test.info().annotations.push({ type: 'step time', description: `${step}: ${(ms / 1000).toFixed(1)} s` });
  return ms;
}

test('Bluetooth switched off and on again on a pair', async ({ testDeviceA, testDeviceB }) => {
  test.skip(
    testDeviceA.kind === 'desktop-process' || testDeviceB.kind === 'desktop-process',
    'needs two radios: run on virtual phones or physical devices',
  );
  test.setTimeout(10 * 60_000);

  const identityA = await testDeviceA.invoke<Identity>('device_get_identity');
  const identityB = await testDeviceB.invoke<Identity>('device_get_identity');
  const pageA = new DevicePage(testDeviceA.page, STEP_TIMEOUT_MS);
  const pageB = new DevicePage(testDeviceB.page, STEP_TIMEOUT_MS);
  await Promise.all([pageA.open(identityB.device_id), pageB.open(identityA.device_id)]);
  const rowA = pageA.channel(ChannelKind.Bluetooth);
  const rowB = pageB.channel(ChannelKind.Bluetooth);
  await rowB.waitForColor(ChannelColor.Green);

  // ADR-0011 D3: with the switch off on testDeviceB, testDeviceB rejects the
  // session, and the row of testDeviceA, still on, turns grey.
  await rowB.switchOff();
  const switchedOff = Date.now();
  await rowA.waitForColor(ChannelColor.Grey, FOLLOW_TIMEOUT_MS);
  note(`${testDeviceA.slug} grey after the switch-off`, switchedOff);

  // The clock starts when the second test device switches on.
  await rowB.switchOn();
  const switchedOn = Date.now();
  const [greenA, greenB] = await Promise.all([
    rowA.waitForColor(ChannelColor.Green, STEP_TIMEOUT_MS).then(() => note(`${testDeviceA.slug} green`, switchedOn)),
    rowB.waitForColor(ChannelColor.Green, STEP_TIMEOUT_MS).then(() => note(`${testDeviceB.slug} green`, switchedOn)),
  ]);
  expect(greenA, `${testDeviceA.slug} green in the standard time`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);
  expect(greenB, `${testDeviceB.slug} green in the standard time`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);
});
