import { test, TestDevicePlatform } from '../test-device.ts';
import { DevicePage } from '../../components/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * Bluetooth switched off and on again on a pair (ADR-0011): a channel is a
 * switch on each device. The second test device (testDeviceB) switches
 * Bluetooth off; it rejects the session, and the row of the first test device
 * (testDeviceA), still on, turns grey. testDeviceB switches it on again, and
 * the row turns green on both. testDeviceA is then in its rejection wait
 * (`REJECTED_WAIT_TIME`, 30 s) and does not connect, so testDeviceB connects
 * to it once (`connect_once_after_switch_on`), with no answer-first wait, also
 * when testDeviceB has the larger device id.
 *
 * The 10 s standard (`docs/architecture/communication.md`) is the target from
 * the moment testDeviceB switches on until each row is green. A row that takes
 * longer gives a warning, not a failure (the decision of the user); the test
 * fails only if a row is not green in STEP_TIMEOUT_MS. The times of the other
 * steps are recorded as annotations.
 *
 * Starts from a pair that has Bluetooth on (run `bluetooth-pair-from-zero`
 * first, or pair by hand). Needs two Bluetooth stacks; run as in that spec
 * (`specs/e2e/device-support/tests/bluetooth-pair-from-zero.spec.ts`).
 */

const DISCOVERY_STANDARD_MS = 10_000;
const STEP_TIMEOUT_MS = 120_000;
/** How long the first test device can take to see the session end. */
const FOLLOW_TIMEOUT_MS = 60_000;
interface Identity {
  device_id: string;
}

function recordStepTime(step: string, startedAt: number): number {
  const ms = Date.now() - startedAt;
  test.info().annotations.push({ type: 'step time', description: `${step}: ${(ms / 1000).toFixed(1)} s` });
  return ms;
}

/** Records a warning, not a failure, when a step takes longer than the 10 s standard. */
function warnIfSlowerThanStandard(step: string, ms: number): void {
  if (ms <= DISCOVERY_STANDARD_MS) return;
  const description = `${step} in ${ms} ms, longer than the ${DISCOVERY_STANDARD_MS} ms standard`;
  test.info().annotations.push({ type: 'warning', description });
  console.warn(`warning: ${description}`);
}

test('Bluetooth switched off and on again on a pair', async ({ testDeviceA, testDeviceB }) => {
  test.skip(
    testDeviceA.isVirtual(TestDevicePlatform.Linux) || testDeviceB.isVirtual(TestDevicePlatform.Linux),
    'needs two Bluetooth stacks: virtual Android or the physical source',
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
  recordStepTime(`${testDeviceA.slug} grey after the switch-off`, switchedOff);

  // The clock starts when the second test device switches on.
  await rowB.switchOn();
  const switchedOn = Date.now();
  const [greenA, greenB] = await Promise.all([
    rowA.waitForColor(ChannelColor.Green, STEP_TIMEOUT_MS).then(() => recordStepTime(`${testDeviceA.slug} green`, switchedOn)),
    rowB.waitForColor(ChannelColor.Green, STEP_TIMEOUT_MS).then(() => recordStepTime(`${testDeviceB.slug} green`, switchedOn)),
  ]);
  warnIfSlowerThanStandard(`${testDeviceA.slug} green`, greenA);
  warnIfSlowerThanStandard(`${testDeviceB.slug} green`, greenB);
});
