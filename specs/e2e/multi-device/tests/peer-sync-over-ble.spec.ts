import { test, expect } from '../test-device.ts';
import type { TestDevice } from '../test-device.ts';
import {
  ensureBlePairedTestDevices,
  expectNetworkChannelUnavailable,
  waitForBluetoothRowConnectedInUi,
  waitForGreenChannel,
} from '../helpers/ble-sync.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';
import { ChannelKind } from '../../../../src/utils/channel.ts';

const TIMEOUT_MS = 60_000;

interface Quest {
  id: string;
  title: string;
}

/**
 * Happy-path-only Phase 1 of the BLE e2e lane: two real `fini-app`
 * processes, network transport made genuinely unavailable
 * (`FINI_E2E_TRANSPORT=ble` -> `FINI_DISCOVERY_DISABLED=1`), dial the real
 * `ble.rs` code path (search coordinator, peripheral accept, exchange) against
 * a cross-process mock radio instead of hardware -- the same acceptance
 * shape the network lane proves for its own channel, one layer
 * more real. See `helpers/ble-sync.ts`, `specs/e2e/transports.md`, and
 * `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`.
 *
 * Deliberately stronger than a bare transport-selection check: proves both
 * sides see each other (green, ADR-0008 D9), and that a
 * *single* quest converges correctly when edited from both ends, not just
 * that traffic flows one direction.
 *
 * Quest convergence is asserted via `get_quests` (backend truth), not by
 * polling the Focus view's DOM: the actual sync -- create, deliver, edit,
 * re-deliver -- was confirmed to complete in ~1-2s every time during this
 * spec's development; what was unreliable was the Focus view picking up
 * the change fast enough under repeated polling, a frontend-refetch timing
 * question this transport lane has no reason to answer. `get_quests` is
 * strictly more precise anyway (exact id, not scraped text) for verifying
 * "the same entity converged."
 */
test('happy-path-BLE: both sides go green over BLE alone, and a single quest converges both ways', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await ensureBlePairedTestDevices([testDeviceA, testDeviceB]);

  // Peer-scoped, so the same spec covers both lanes: spawned testDevices see no
  // presence at all, while on hardware this pair has no Network channel on.
  await expectNetworkChannelUnavailable(testDeviceA, syncedB.identity.device_id);
  await expectNetworkChannelUnavailable(testDeviceB, syncedA.identity.device_id);

  // Green on both, first as the Device page draws it -- opening the page is
  // what runs the status search that hears the peer (ADR-0008 D12) -- then
  // as the backend reports it. An idle pair holds no session, so presence,
  // not a session, is what green means (D9).
  await waitForBluetoothRowConnectedInUi(testDeviceA, syncedB.identity.device_id);
  await waitForBluetoothRowConnectedInUi(testDeviceB, syncedA.identity.device_id);
  await waitForGreenChannel(testDeviceA, syncedB.identity.device_id);
  await waitForGreenChannel(testDeviceB, syncedA.identity.device_id);

  // Saving the mapping is the first work, and it opens the first exchange --
  // over Bluetooth, the only channel these two can use.
  await ensurePersonalSpaceSync(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
    ChannelKind.Bluetooth,
  );
  await expectNoIncomingSpaceSyncDialog(testDeviceA);
  await expectNoIncomingSpaceSyncDialog(testDeviceB);

  // `ensurePersonalSpaceSync` only proves the mapping was recorded/approved
  // locally, not that a full sync round-trip has happened -- matches
  // `zz-personal-space-live-quest-sync.spec.ts`'s own use of this same call
  // as an extra readiness gate before creating anything, so a quest created
  // immediately after approval doesn't race the very first backfill tick.
  await waitForPersonalLastSyncedLabel(testDeviceB, syncedA.identity.device_id);
  await sleep(1_100);

  const originalTitle = `BLE sync ${Date.now()}`;
  const editedTitle = `${originalTitle} (edited on B)`;

  const createdOnA = await createQuestViaChat(testDeviceA, originalTitle);
  const questOnB = await waitForQuestConverged(testDeviceA, testDeviceB, originalTitle);
  expect(questOnB.id, 'the synced quest must be the same entity, not a new one').toBe(createdOnA.id);

  await testDeviceB.invoke('update_quest', { id: questOnB.id, input: { title: editedTitle } });

  const questOnA = await waitForQuestConverged(testDeviceB, testDeviceA, editedTitle);
  expect(questOnA.id, 'the edit must land on the same entity, not create a second quest').toBe(createdOnA.id);
});

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function openFocus(testDevice: TestDevice): Promise<void> {
  await testDevice.page.click('nav.nav a[href="#/main"]');
  await testDevice.page.waitForSelector('[data-testid="chat-input"]', TIMEOUT_MS);
}

/** Creates a quest through the real chat-input UI, then confirms it landed locally via `get_quests`. */
async function createQuestViaChat(testDevice: TestDevice, title: string): Promise<Quest> {
  await openFocus(testDevice);
  await testDevice.page.fill('[data-testid="chat-input"]', title);
  await testDevice.page.click('[data-testid="chat-submit"]');

  return findQuestByTitle(testDevice, title);
}

/** Ticks both testDevices and polls the receiver's own `get_quests` until the given title lands there. */
async function waitForQuestConverged(sender: TestDevice, receiver: TestDevice, title: string): Promise<Quest> {
  return pollUntil(`${receiver.slug} converges on quest titled "${title}"`, async () => {
    await sender.invoke('space_sync_tick');
    await receiver.invoke('space_sync_tick');
    const quests = await receiver.invoke<Quest[]>('get_quests');
    return quests.find((quest) => quest.title === title) ?? false;
  }, TIMEOUT_MS, 1_000);
}

async function findQuestByTitle(testDevice: TestDevice, title: string): Promise<Quest> {
  return pollUntil(`${testDevice.slug} finds quest titled "${title}" via get_quests`, async () => {
    const quests = await testDevice.invoke<Quest[]>('get_quests');
    return quests.find((quest) => quest.title === title) ?? false;
  }, TIMEOUT_MS, 1_000);
}
