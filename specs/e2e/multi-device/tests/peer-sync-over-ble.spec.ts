import { test, expect } from '../test-device.ts';
import type { TestDevice } from '../test-device.ts';
import {
  pairTestDevicesOverBluetooth,
  expectNetworkChannelUnavailable,
  waitForBluetoothRowConnectedInUi,
  waitForGreenChannel,
} from '../helpers/ble-sync.ts';
import {
  syncPersonalSpace,
  expectNoIncomingSpaceSyncDialog,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';
import { ChannelKind } from '../../../../src/utils/channel.ts';

// Virtual Linux test devices on the ble-gatt mock radio, with the Network
// channel off. A worker option: Playwright starts a new worker for this file.
test.use({ testDeviceChannel: ChannelKind.Bluetooth });

const TIMEOUT_MS = 60_000;

interface Quest {
  id: string;
  title: string;
}

/**
 * A happy path of `multi-device` over Bluetooth. Two `fini-app` processes
 * have the Network channel off (`testDeviceChannel: ChannelKind.Bluetooth`
 * gives `FINI_DISCOVERY_DISABLED=1`). The Bluetooth code of `ble.rs` runs
 * (the search, the peripheral accept, the exchange) on the mock radio of
 * `ble-gatt`, not on a hardware radio. See `helpers/ble-sync.ts`,
 * `docs/e2e.md` ("What each project proves for each channel"), and
 * `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`.
 *
 * The test proves more than the channel order: the two sides hear each other
 * (green, ADR-0008 D9), and one quest gets the same value on the two sides
 * when each side changes it.
 *
 * The test reads the quests with `get_quests` (the database of the app), not
 * from the Focus view. The sync (create, send, edit, send again) took 1 to
 * 2 s in each run. The Focus view sometimes showed the change later; that
 * is a question for the frontend, not for this test. `get_quests` also
 * compares the id, not only the text.
 */
test('happy-path-BLE: both sides go green over BLE alone, and a single quest converges both ways', async ({
  testDeviceA,
  testDeviceB,
}) => {
  const [syncedA, syncedB] = await pairTestDevicesOverBluetooth([testDeviceA, testDeviceB]);

  // For each peer, so the spec works on each source: a virtual Linux test
  // device hears no device, and on hardware the pair has no Network channel on.
  await expectNetworkChannelUnavailable(testDeviceA, syncedB.identity.device_id);
  await expectNetworkChannelUnavailable(testDeviceB, syncedA.identity.device_id);

  // Green on the two sides: first on the Device page, then from the Rust
  // side. The open page runs the status search that hears the peer
  // (ADR-0008 D12). A pair with no work has no session, so green means that
  // the app hears the peer, not that a session is open (D9).
  await waitForBluetoothRowConnectedInUi(testDeviceA, syncedB.identity.device_id);
  await waitForBluetoothRowConnectedInUi(testDeviceB, syncedA.identity.device_id);
  await waitForGreenChannel(testDeviceA, syncedB.identity.device_id);
  await waitForGreenChannel(testDeviceB, syncedA.identity.device_id);

  // The saved mapping is the first work, and it opens the first exchange,
  // over Bluetooth: the only channel that these two can use.
  await syncPersonalSpace(
    testDeviceA,
    syncedB.identity.device_id,
    testDeviceB,
    syncedA.identity.device_id,
    ChannelKind.Bluetooth,
  );
  await expectNoIncomingSpaceSyncDialog(testDeviceA);
  await expectNoIncomingSpaceSyncDialog(testDeviceB);

  // `syncPersonalSpace` proves only that each device recorded the
  // mapping, not that a sync went in the two directions. Thus the test waits
  // for the first sync, as `zz-personal-space-live-quest-sync.spec.ts` does,
  // before it makes a quest. Else the new quest and the first sync can occur
  // at the same time.
  await waitForPersonalLastSyncedLabel(testDeviceB, syncedA.identity.device_id);
  await sleep(1_100);

  const originalTitle = `BLE sync ${Date.now()}`;
  const editedTitle = `${originalTitle} (edited on B)`;

  const createdOnA = await createQuestInChat(testDeviceA, originalTitle);
  const questOnB = await waitForQuestSynced(testDeviceA, testDeviceB, originalTitle);
  expect(questOnB.id, 'the synced quest has the same id: it is not a new quest').toBe(createdOnA.id);

  await testDeviceB.invoke('update_quest', { id: questOnB.id, input: { title: editedTitle } });

  const questOnA = await waitForQuestSynced(testDeviceB, testDeviceA, editedTitle);
  expect(questOnA.id, 'the edit changes the same quest: there is no second quest').toBe(createdOnA.id);
});

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function openFocus(testDevice: TestDevice): Promise<void> {
  await testDevice.page.click('nav.nav a[href="#/main"]');
  await testDevice.page.waitForSelector('[data-testid="chat-input"]', TIMEOUT_MS);
}

/** Makes a quest through the chat input of the app, then finds it with `get_quests`. */
async function createQuestInChat(testDevice: TestDevice, title: string): Promise<Quest> {
  await openFocus(testDevice);
  await testDevice.page.fill('[data-testid="chat-input"]', title);
  await testDevice.page.click('[data-testid="chat-submit"]');

  return findQuestByTitle(testDevice, title);
}

/** Runs a sync tick on the two test devices until `get_quests` of the receiver has the title. */
async function waitForQuestSynced(sender: TestDevice, receiver: TestDevice, title: string): Promise<Quest> {
  return pollUntil(`${receiver.slug} has the quest "${title}"`, async () => {
    await sender.invoke('space_sync_tick');
    await receiver.invoke('space_sync_tick');
    const quests = await receiver.invoke<Quest[]>('get_quests');
    return quests.find((quest) => quest.title === title) ?? false;
  }, TIMEOUT_MS, 1_000);
}

async function findQuestByTitle(testDevice: TestDevice, title: string): Promise<Quest> {
  return pollUntil(`${testDevice.slug} has the quest "${title}" in get_quests`, async () => {
    const quests = await testDevice.invoke<Quest[]>('get_quests');
    return quests.find((quest) => quest.title === title) ?? false;
  }, TIMEOUT_MS, 1_000);
}
