import { test, expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { ensureSyncedActors, type SyncedActor } from '../helpers/device-sync.ts';
import { openDeviceDetailsFromSettings } from '../helpers/personal-sync.ts';
import {
  addChannelViaDialog,
  channelState,
  confirmSetupDialog,
  unlinkChannel,
  waitForChannelState,
  ChannelKind,
  ChannelState,
} from '../helpers/device-page.ts';

/**
 * Adding a Bluetooth channel to a device that is already paired, taking it
 * away again, and adding it again -- over and over.
 *
 * Needs a radio and a second channel, so it runs only against real devices
 * as external actors (see `runsHere` for why neither container lane can):
 *
 *   FINI_E2E_ACTORS=desktop,phone \
 *   FINI_E2E_EXTERNAL_ACTORS=desktop=9224,phone=9223 \
 *   npx playwright test --config specs/e2e/playwright.config.ts \
 *     --project actors -g "Bluetooth channel"
 *
 * `FINI_E2E_BT_CYCLES` sets how many add/unlink rounds run (default 3).
 *
 * Found by doing it by hand on a laptop and a Pixel, where each of these
 * broke the flow at least once: the phone's scan returned nothing (a missing
 * manifest flag), the add-mode candidate scan dialled the peer while the
 * setup said hello to it, and BlueZ stopped advertising after a central had
 * connected and left. A single add passes without most of them; the repeats
 * are what exposes them.
 */

const CYCLES = Number(process.env.FINI_E2E_BT_CYCLES ?? 3);

/**
 * Setup waits on a search that hears the peer's advertisement, and the
 * peer's address rotates, so on hardware it measured 30 to 100 seconds.
 */
const SETUP_TIMEOUT_MS = 180_000;

/**
 * An unlink is pushed to the peer on its next exchange, which measured 25
 * to 50 seconds on hardware.
 */
const UNLINK_PROPAGATION_TIMEOUT_MS = 120_000;

/**
 * Spawned actors in the default lane have no radio (the container runs no
 * `bluetoothd`; `device-page.spec.ts` covers that refusal).
 *
 * The BLE lane (`FINI_E2E_TRANSPORT=ble`, a mock radio) has one, but runs
 * with Network discovery off and pairs over Bluetooth alone, so Bluetooth is
 * the pair's only channel. An unlink notice rides the next exchange over a
 * channel that is still `On` (ADR-0008 D14; `request_exchanges` never uses
 * one that is not), so once Bluetooth is unlinked there is nothing left to
 * carry it and the peer never follows. Tried there: the first unlink timed
 * out waiting for the other side. Real devices keep Network alongside.
 */
function runsHere(actor: E2EActor): boolean {
  return actor.kind === 'external';
}

async function pairForTest(actorA: E2EActor, actorB: E2EActor): Promise<SyncedActor[]> {
  return ensureSyncedActors([actorA, actorB], { pairViaUi: true });
}

/**
 * Both devices end with no Bluetooth channel. Unlinking on one device is
 * enough: the other must follow without anyone touching it.
 */
async function removeBluetoothChannel(
  actorA: E2EActor,
  peerOfA: string,
  actorB: E2EActor,
  peerOfB: string,
): Promise<void> {
  if ((await channelState(actorA, peerOfA, ChannelKind.Bluetooth)) !== ChannelState.None) {
    await unlinkChannel(actorA, peerOfA, ChannelKind.Bluetooth);
  }
  await waitForChannelState(
    actorB,
    peerOfB,
    ChannelKind.Bluetooth,
    ChannelState.None,
    UNLINK_PROPAGATION_TIMEOUT_MS,
  );
}

/** Add on both devices at once, as two people do, then press OK on both. */
async function addBluetoothChannel(
  actorA: E2EActor,
  peerOfA: string,
  actorB: E2EActor,
  peerOfB: string,
): Promise<void> {
  await Promise.all([
    openDeviceDetailsFromSettings(actorA, peerOfA),
    openDeviceDetailsFromSettings(actorB, peerOfB),
  ]);
  const startedAt = Date.now();
  const [outcomeA, outcomeB] = await Promise.all([
    addChannelViaDialog(actorA, ChannelKind.Bluetooth, SETUP_TIMEOUT_MS),
    addChannelViaDialog(actorB, ChannelKind.Bluetooth, SETUP_TIMEOUT_MS),
  ]);
  // Printed so a run shows how long the person would have stared at the
  // dialog, not only whether it finished.
  console.log(`bluetooth setup ready on both devices after ${((Date.now() - startedAt) / 1000).toFixed(1)}s`);
  expect(outcomeA, `${actorA.slug} setup: ${outcomeA.failure}`).toMatchObject({ ready: true });
  expect(outcomeB, `${actorB.slug} setup: ${outcomeB.failure}`).toMatchObject({ ready: true });

  await Promise.all([confirmSetupDialog(actorA), confirmSetupDialog(actorB)]);
  await waitForChannelState(actorA, peerOfA, ChannelKind.Bluetooth, ChannelState.On);
  await waitForChannelState(actorB, peerOfB, ChannelKind.Bluetooth, ChannelState.On);
}

test('Bluetooth channel: add, unlink, add again, repeatedly', async ({ actorA, actorB }) => {
  test.skip(
    !runsHere(actorA) || !runsHere(actorB),
    'needs a radio and a second channel: run against real devices as external actors',
  );
  test.setTimeout(CYCLES * (SETUP_TIMEOUT_MS + UNLINK_PROPAGATION_TIMEOUT_MS) + 120_000);

  const [syncedA, syncedB] = await pairForTest(actorA, actorB);
  const peerOfA = syncedB.identity.device_id;
  const peerOfB = syncedA.identity.device_id;

  for (let cycle = 1; cycle <= CYCLES; cycle += 1) {
    await removeBluetoothChannel(actorA, peerOfA, actorB, peerOfB);
    await addBluetoothChannel(actorA, peerOfA, actorB, peerOfB);
    expect(
      await channelState(actorA, peerOfA, ChannelKind.Bluetooth),
      `cycle ${cycle}: ${actorA.slug} has Bluetooth on`,
    ).toBe(ChannelState.On);
    expect(
      await channelState(actorB, peerOfB, ChannelKind.Bluetooth),
      `cycle ${cycle}: ${actorB.slug} has Bluetooth on`,
    ).toBe(ChannelState.On);
  }
});
