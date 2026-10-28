import { expect, test } from '../test-device.ts';
import type { TestDevice } from '../test-device.ts';
import { AddDeviceDialog } from '../../components/add-device-dialog.ts';
import { DevicePage } from '../../components/device-page.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

/**
 * A new pair over Bluetooth, from zero, the way a person makes it: both
 * devices open Add device -> Bluetooth, each lists the other, Pair, Accept,
 * the code, Done, and the Bluetooth row turns green on both.
 *
 * The 10 s standard (`docs/architecture/communication.md`) is asserted for the
 * listing: from the moment both devices search until each lists the other.
 * The other steps have no fixed limit; their times are recorded as test
 * annotations, so a run shows how long a person waits at each step.
 *
 * Needs two real radios: run against devices as external testDevices, e.g. the
 * desktop and phone dev apps (`make desktop-debug-dev`, `make android-dev`
 * and `adb forward tcp:9223 tcp:9223`):
 *   FINI_E2E_ACTORS=desktop,phone \
 *   FINI_E2E_EXTERNAL_ACTORS=desktop=9224,phone=9223 \
 *   npx playwright test --config specs/e2e/playwright.config.ts \
 *     --project testDevices bluetooth-pair-from-zero
 */

const DISCOVERY_STANDARD_MS = 10_000;
const STEP_TIMEOUT_MS = 120_000;

interface Identity {
  device_id: string;
  hostname: string;
}

interface PairedDevice {
  peer_device_id: string;
}

function note(step: string, startedAt: number): number {
  const ms = Date.now() - startedAt;
  test.info().annotations.push({ type: 'step time', description: `${step}: ${(ms / 1000).toFixed(1)} s` });
  return ms;
}

interface ChannelStatus {
  kind: ChannelKind;
  color: ChannelColor;
}

/** Waits until `testDevice`'s Bluetooth channel with `peerDeviceId` is green. */
async function waitForGreen(testDevice: TestDevice, peerDeviceId: string): Promise<void> {
  const deadline = Date.now() + STEP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const statuses = await testDevice.invoke<ChannelStatus[]>('device_channel_statuses', { peerDeviceId });
    if (statuses.find((status) => status.kind === ChannelKind.Bluetooth)?.color === ChannelColor.Green) return;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${testDevice.slug} Bluetooth with ${peerDeviceId} not green in ${STEP_TIMEOUT_MS} ms`);
}

/** Removes the pair on `testDevice`'s side through its device page, if it has one. */
async function unpairIfPaired(testDevice: TestDevice, peer: Identity): Promise<void> {
  const paired = await testDevice.invoke<PairedDevice[]>('device_paired_devices');
  if (!paired.some((device) => device.peer_device_id === peer.device_id)) return;
  const page = new DevicePage(testDevice.page, STEP_TIMEOUT_MS);
  await page.open(peer.device_id);
  await page.unlinkDevice();
}

test('a new pair over Bluetooth, from zero to green', async ({ testDeviceA, testDeviceB }) => {
  test.skip(
    testDeviceA.kind === 'desktop-process' || testDeviceB.kind === 'desktop-process',
    'needs two radios: run on virtual phones or physical devices',
  );
  test.setTimeout(10 * 60_000);

  const identityA = await testDeviceA.invoke<Identity>('device_get_identity');
  const identityB = await testDeviceB.invoke<Identity>('device_get_identity');
  await unpairIfPaired(testDeviceA, identityB);
  await unpairIfPaired(testDeviceB, identityA);

  const dialogA = new AddDeviceDialog(testDeviceA.page, STEP_TIMEOUT_MS);
  const dialogB = new AddDeviceDialog(testDeviceB.page, STEP_TIMEOUT_MS);
  await dialogA.openSearch(ChannelKind.Bluetooth);
  await dialogB.openSearch(ChannelKind.Bluetooth);

  const searching = Date.now();
  const [listedByA, listedByB] = await Promise.all([
    dialogA.waitForDevice(identityB.hostname).then(() => note(`${testDeviceA.slug} lists ${testDeviceB.slug}`, searching)),
    dialogB.waitForDevice(identityA.hostname).then(() => note(`${testDeviceB.slug} lists ${testDeviceA.slug}`, searching)),
  ]);
  expect(listedByA, `${testDeviceA.slug} lists ${testDeviceB.slug} within the standard`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);
  expect(listedByB, `${testDeviceB.slug} lists ${testDeviceA.slug} within the standard`).toBeLessThanOrEqual(DISCOVERY_STANDARD_MS);

  const pairPressed = Date.now();
  await dialogA.pair(identityB.hostname);
  await dialogB.waitForRequest();
  note(`Pair -> request on ${testDeviceB.slug}`, pairPressed);

  await dialogB.accept();
  const code = await dialogA.code();
  expect(code).toMatch(/^\d{6}$/);

  const codeTyped = Date.now();
  await dialogB.typeCode(code);
  await Promise.all([dialogA.waitForPaired(), dialogB.waitForPaired()]);
  note('code -> Paired on both', codeTyped);

  // The first session runs over the pairing's link, so the channel turns
  // green while the dialogs still show Paired. Read from the backend: the
  // row is behind the dialog.
  const paired = Date.now();
  await Promise.all([
    waitForGreen(testDeviceA, identityB.device_id).then(() => note(`Paired -> ${testDeviceA.slug} green`, paired)),
    waitForGreen(testDeviceB, identityA.device_id).then(() => note(`Paired -> ${testDeviceB.slug} green`, paired)),
  ]);

  await Promise.all([dialogA.done(), dialogB.done()]);
  const done = Date.now();
  const pageA = new DevicePage(testDeviceA.page, STEP_TIMEOUT_MS);
  const pageB = new DevicePage(testDeviceB.page, STEP_TIMEOUT_MS);
  await Promise.all([pageA.open(identityB.device_id), pageB.open(identityA.device_id)]);
  await Promise.all([
    pageA.channel(ChannelKind.Bluetooth).waitForColor(ChannelColor.Green).then(() => note(`Done -> ${testDeviceA.slug} green`, done)),
    pageB.channel(ChannelKind.Bluetooth).waitForColor(ChannelColor.Green).then(() => note(`Done -> ${testDeviceB.slug} green`, done)),
  ]);
});
