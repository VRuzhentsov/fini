import { expect, test } from '../fixtures.ts';
import { DevicePage } from '../../components/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * Bluetooth removed and added again on a pair, the way a person does it on
 * the second device (actorB): switch Bluetooth off, Unlink, Add. The first
 * device (actorA) follows the removal by itself and offers Add: its next
 * session connect gets `CLOSE_NO_CHANNEL` from actorB, which advertises
 * again for the setup. The person presses Add there too, and OK on both
 * turns the row green.
 *
 * The 10 s standard (`docs/architecture/communication.md`) is asserted for
 * the search: from the moment both setup dialogs are open until OK is
 * available on both. The other steps' times are recorded as annotations.
 *
 * Starts from a pair that has Bluetooth on (run `bluetooth-pair-from-zero`
 * first, or pair by hand). Needs two real radios; run as in that spec.
 */

const DISCOVERY_STANDARD_MS = 10_000;
const STEP_TIMEOUT_MS = 120_000;
/**
 * How long the first device may take to follow the removal: its next session
 * connect, which waits out the refusal at switch-off (30 s).
 */
const FOLLOW_TIMEOUT_MS = 60_000;

interface Identity {
  device_id: string;
}

function note(step: string, startedAt: number): number {
  const ms = Date.now() - startedAt;
  test.info().annotations.push({ type: 'step time', description: `${step}: ${(ms / 1000).toFixed(1)} s` });
  return ms;
}

test('Bluetooth removed and added again on a pair', async ({ actorA, actorB }) => {
  test.skip(
    actorA.kind !== 'external' || actorB.kind !== 'external',
    'needs two radios: run against real devices as external actors',
  );
  test.setTimeout(10 * 60_000);

  const identityA = await actorA.invoke<Identity>('device_connection_get_identity');
  const identityB = await actorB.invoke<Identity>('device_connection_get_identity');
  const pageA = new DevicePage(actorA.page, STEP_TIMEOUT_MS);
  const pageB = new DevicePage(actorB.page, STEP_TIMEOUT_MS);
  await Promise.all([pageA.open(identityB.device_id), pageB.open(identityA.device_id)]);
  const rowA = pageA.channel(ChannelKind.Bluetooth);
  const rowB = pageB.channel(ChannelKind.Bluetooth);
  await rowB.waitForColor(ChannelColor.Green);

  await rowB.switchOff();
  const removed = Date.now();
  await rowB.unlink();
  const setupB = await rowB.add();
  await rowA.waitForAdd(FOLLOW_TIMEOUT_MS);
  note(`${actorA.slug} follows the removal`, removed);

  const setupA = await rowA.add();
  const searching = Date.now();
  const [readyB, readyA] = await Promise.all([
    setupB.waitForOk(STEP_TIMEOUT_MS).then(() => note(`${actorB.slug} OK available`, searching)),
    setupA.waitForOk(STEP_TIMEOUT_MS).then(() => note(`${actorA.slug} OK available`, searching)),
  ]);
  expect(readyA, `${actorA.slug} finds ${actorB.slug} within the standard`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);
  expect(readyB, `${actorB.slug} finds ${actorA.slug} within the standard`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);

  await Promise.all([setupB.ok(), setupA.ok()]);
  const ok = Date.now();
  await Promise.all([
    rowA.waitForColor(ChannelColor.Green).then(() => note(`OK -> ${actorA.slug} green`, ok)),
    rowB.waitForColor(ChannelColor.Green).then(() => note(`OK -> ${actorB.slug} green`, ok)),
  ]);
});
