import { test, expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { ensureBlePairedActors } from '../helpers/ble-sync.ts';
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
 * Needs a radio. Spawned actors in the default lane have none (the container
 * runs no `bluetoothd`; `device-page.spec.ts` covers that refusal), so this
 * runs in the BLE lane (`FINI_E2E_TRANSPORT=ble`, a mock radio) or against
 * real devices as external actors:
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

function hasRadio(actor: E2EActor): boolean {
  return actor.kind === 'external' || process.env.FINI_E2E_TRANSPORT === 'ble';
}

async function pairForTest(actorA: E2EActor, actorB: E2EActor): Promise<SyncedActor[]> {
  // The BLE lane runs with discovery off, so nothing can be paired over the
  // network there; it starts from a pair that already has Bluetooth.
  if (process.env.FINI_E2E_TRANSPORT === 'ble') {
    return ensureBlePairedActors([actorA, actorB]);
  }
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
  const [outcomeA, outcomeB] = await Promise.all([
    addChannelViaDialog(actorA, ChannelKind.Bluetooth, SETUP_TIMEOUT_MS),
    addChannelViaDialog(actorB, ChannelKind.Bluetooth, SETUP_TIMEOUT_MS),
  ]);
  expect(outcomeA, `${actorA.slug} setup: ${outcomeA.failure}`).toMatchObject({ ready: true });
  expect(outcomeB, `${actorB.slug} setup: ${outcomeB.failure}`).toMatchObject({ ready: true });

  await Promise.all([confirmSetupDialog(actorA), confirmSetupDialog(actorB)]);
  await waitForChannelState(actorA, peerOfA, ChannelKind.Bluetooth, ChannelState.On);
  await waitForChannelState(actorB, peerOfB, ChannelKind.Bluetooth, ChannelState.On);
}

test('Bluetooth channel: add, unlink, add again, repeatedly', async ({ actorA, actorB }) => {
  test.skip(
    !hasRadio(actorA) || !hasRadio(actorB),
    'needs a radio: run in the BLE lane or against real devices as external actors',
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
