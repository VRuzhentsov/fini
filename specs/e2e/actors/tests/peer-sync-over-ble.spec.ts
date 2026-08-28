import { test, expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import {
  ensureBlePairedActors,
  expectNetworkTransportUnavailable,
  waitForBleSession,
  waitForBluetoothRowConnectedInUi,
  waitForGreenTransport,
} from '../helpers/ble-sync.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  waitForPersonalLastSyncedLabel,
} from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';

const TIMEOUT_MS = 60_000;

interface Quest {
  id: string;
  title: string;
}

/**
 * Happy-path-only Phase 1 of the BLE e2e lane: two real `fini-app`
 * processes, network transport made genuinely unavailable
 * (`FINI_E2E_TRANSPORT=ble` -> `FINI_DISCOVERY_DISABLED=1`), dial the real
 * `ble.rs` code path (dial loop, peripheral accept, session claim) against
 * a cross-process mock radio instead of hardware -- the same acceptance
 * shape `peer-sync-over-sim.spec.ts` proves for the Sim stand-in, one layer
 * more real. See `helpers/ble-sync.ts`, `specs/e2e/transports.md`, and
 * `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`.
 *
 * Deliberately stronger than a bare transport-selection check: proves both
 * sides reach a genuinely healthy (ping/ack-proven) state, and that a
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
test('peer session establishes over BLE, both sides go green, and a single quest converges both ways', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureBlePairedActors([actorA, actorB]);

  await expectNetworkTransportUnavailable(actorA);
  await expectNetworkTransportUnavailable(actorB);

  await waitForBleSession(actorA);
  await waitForBleSession(actorB);

  const kindOnA = await actorA.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedB.identity.device_id,
  });
  const kindOnB = await actorB.invoke<string>('device_connection_session_transport', {
    peerDeviceId: syncedA.identity.device_id,
  });
  expect(kindOnA).toBe('bluetooth');
  expect(kindOnB).toBe('bluetooth');

  // Green on both -- the backend's ping/ack-proven signal, and the UI row
  // that actually renders it (and never regresses through "Still
  // connecting..." on the way there). See `waitForGreenTransport`'s doc
  // comment for why `code === null`, not just "a session exists", is the
  // bar here. This is the real regression guard this e2e lane exists for,
  // so it stays UI-asserted, unlike the quest-convergence checks below.
  await waitForGreenTransport(actorA, syncedB.identity.device_id);
  await waitForGreenTransport(actorB, syncedA.identity.device_id);
  await waitForBluetoothRowConnectedInUi(actorA, syncedB.identity.device_id);
  await waitForBluetoothRowConnectedInUi(actorB, syncedA.identity.device_id);

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);
  await expectNoIncomingSpaceSyncDialog(actorA);
  await expectNoIncomingSpaceSyncDialog(actorB);

  // `ensurePersonalSpaceSync` only proves the mapping was recorded/approved
  // locally, not that a full sync round-trip has happened -- matches
  // `zz-personal-space-live-quest-sync.spec.ts`'s own use of this same call
  // as an extra readiness gate before creating anything, so a quest created
  // immediately after approval doesn't race the very first backfill tick.
  await waitForPersonalLastSyncedLabel(actorB, syncedA.identity.device_id);
  await sleep(1_100);

  const originalTitle = `BLE sync ${Date.now()}`;
  const editedTitle = `${originalTitle} (edited on B)`;

  const createdOnA = await createQuestViaChat(actorA, originalTitle);
  const questOnB = await waitForQuestConverged(actorA, actorB, originalTitle);
  expect(questOnB.id, 'the synced quest must be the same entity, not a new one').toBe(createdOnA.id);

  await actorB.invoke('update_quest', { id: questOnB.id, input: { title: editedTitle } });

  const questOnA = await waitForQuestConverged(actorB, actorA, editedTitle);
  expect(questOnA.id, 'the edit must land on the same entity, not create a second quest').toBe(createdOnA.id);
});

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function openFocus(actor: E2EActor): Promise<void> {
  await actor.page.click('nav.nav a[href="#/main"]');
  await actor.page.waitForSelector('[data-testid="chat-input"]', TIMEOUT_MS);
}

/** Creates a quest through the real chat-input UI, then confirms it landed locally via `get_quests`. */
async function createQuestViaChat(actor: E2EActor, title: string): Promise<Quest> {
  await openFocus(actor);
  await actor.page.evaluate(`(() => {
    const input = document.querySelector('[data-testid="chat-input"]');
    if (!(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)) {
      throw new Error('chat input text control not found');
    }
    const prototype = input instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
    if (!setter) throw new Error('chat input setter is unavailable');
    setter.call(input, ${JSON.stringify(title)});
    input.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await actor.page.click('[data-testid="chat-submit"]');

  return findQuestByTitle(actor, title);
}

/** Ticks both actors and polls the receiver's own `get_quests` until the given title lands there. */
async function waitForQuestConverged(sender: E2EActor, receiver: E2EActor, title: string): Promise<Quest> {
  return pollUntil(`${receiver.slug} converges on quest titled "${title}"`, async () => {
    await sender.invoke('space_sync_tick');
    await receiver.invoke('space_sync_tick');
    const quests = await receiver.invoke<Quest[]>('get_quests');
    return quests.find((quest) => quest.title === title) ?? false;
  }, TIMEOUT_MS, 1_000);
}

async function findQuestByTitle(actor: E2EActor, title: string): Promise<Quest> {
  return pollUntil(`${actor.slug} finds quest titled "${title}" via get_quests`, async () => {
    const quests = await actor.invoke<Quest[]>('get_quests');
    return quests.find((quest) => quest.title === title) ?? false;
  }, TIMEOUT_MS, 1_000);
}
