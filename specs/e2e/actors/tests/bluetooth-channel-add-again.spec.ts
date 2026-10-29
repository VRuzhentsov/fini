import { expect, test } from '../fixtures.ts';
import { DevicePage } from '../../components/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * Bluetooth switched off and on again on a pair (ADR-0011): a channel is a
 * switch on each device. The second device (actorB) switches Bluetooth off;
 * it refuses the session, and the first device's row (actorA), still on,
 * turns grey. actorB switches it on again, and the row turns green on both.
 *
 * The 10 s standard (`docs/architecture/communication.md`) is asserted from
 * the moment actorB switches on until each row is green. The other steps'
 * times are recorded as annotations.
 *
 * Starts from a pair that has Bluetooth on (run `bluetooth-pair-from-zero`
 * first, or pair by hand). Needs two real radios; run as in that spec.
 */

const DISCOVERY_STANDARD_MS = 10_000;
const STEP_TIMEOUT_MS = 120_000;
/** How long the first device may take to see the session end. */
const FOLLOW_TIMEOUT_MS = 60_000;
interface Identity {
  device_id: string;
}

function note(step: string, startedAt: number): number {
  const ms = Date.now() - startedAt;
  test.info().annotations.push({ type: 'step time', description: `${step}: ${(ms / 1000).toFixed(1)} s` });
  return ms;
}

test('Bluetooth switched off and on again on a pair', async ({ actorA, actorB }) => {
  test.skip(
    actorA.kind !== 'external' || actorB.kind !== 'external',
    'needs two radios: run against real devices as external actors',
  );
  test.setTimeout(10 * 60_000);

  const identityA = await actorA.invoke<Identity>('device_get_identity');
  const identityB = await actorB.invoke<Identity>('device_get_identity');
  const pageA = new DevicePage(actorA.page, STEP_TIMEOUT_MS);
  const pageB = new DevicePage(actorB.page, STEP_TIMEOUT_MS);
  await Promise.all([pageA.open(identityB.device_id), pageB.open(identityA.device_id)]);
  const rowA = pageA.channel(ChannelKind.Bluetooth);
  const rowB = pageB.channel(ChannelKind.Bluetooth);
  await rowB.waitForColor(ChannelColor.Green);

  // ADR-0011 D3: with the switch off on actorB, actorB refuses the session,
  // and actorA's row, still on, turns grey.
  await rowB.switchOff();
  const switchedOff = Date.now();
  await rowA.waitForColor(ChannelColor.Grey, FOLLOW_TIMEOUT_MS);
  note(`${actorA.slug} grey after the switch-off`, switchedOff);

  // The clock starts when the second device switches on.
  await rowB.switchOn();
  const switchedOn = Date.now();
  const [greenA, greenB] = await Promise.all([
    rowA.waitForColor(ChannelColor.Green, STEP_TIMEOUT_MS).then(() => note(`${actorA.slug} green`, switchedOn)),
    rowB.waitForColor(ChannelColor.Green, STEP_TIMEOUT_MS).then(() => note(`${actorB.slug} green`, switchedOn)),
  ]);
  expect(greenA, `${actorA.slug} green within the standard`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);
  expect(greenB, `${actorB.slug} green within the standard`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);
});
