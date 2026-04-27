import { test, expect } from '../fixtures.js';
import type { E2EActor } from '../fixtures.js';
import { ensureSyncedActors } from '../helpers/device-sync.js';
import { pollUntil } from '../helpers/dom.js';

interface SpaceSyncStatus {
  peer_device_id: string;
  mapped_space_ids: string[];
  last_synced_at: string | null;
  last_synced_at_by_space: Record<string, string | null>;
  pending_event_count: number;
  outbox_event_count: number;
  acked_event_count: number;
  seen_event_count: number;
  tombstone_count: number;
}

const PERSONAL_SPACE_ID = '1';
const TIMEOUT_MS = 60_000;

test('device can request Personal space sync and peer confirms it', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  await waitForPeerSession(actorA);
  await waitForPeerSession(actorB);

  await openDeviceDetailsFromSettings(actorA, syncedB.identity.device_id);
  await waitForMappingControlsReady(actorA, PERSONAL_SPACE_ID);

  const mappedBefore = await actorA.invoke<string[]>('space_sync_list_mappings', {
    peerDeviceId: syncedB.identity.device_id,
  });
  expect(mappedBefore).not.toContain(PERSONAL_SPACE_ID);

  await actorA.page.click(spaceCheckboxSelector(PERSONAL_SPACE_ID));
  await actorA.page.waitForSelector('[data-testid="save-space-mappings"]:not([disabled])', TIMEOUT_MS);
  await actorA.page.click('[data-testid="save-space-mappings"]');

  await pollUntil('A persists Personal mapping', async () => {
    const mapped = await actorA.invoke<string[]>('space_sync_list_mappings', {
      peerDeviceId: syncedB.identity.device_id,
    });
    return mapped.includes(PERSONAL_SPACE_ID) || false;
  }, TIMEOUT_MS);

  await actorB.page.waitForSelector(
    `[data-testid="incoming-space-sync-dialog"][data-dialog-kind="approve"][data-peer-device-id="${cssString(syncedA.identity.device_id)}"][data-space-count="1"]`,
    TIMEOUT_MS,
  );
  await actorB.page.click('[data-testid="approve-space-sync"]');
  await waitForApproveDialogToClose(actorB);

  await pollUntil('B persists approved Personal mapping', async () => {
    await tickActors([actorA, actorB]);
    const mapped = await actorB.invoke<string[]>('space_sync_list_mappings', {
      peerDeviceId: syncedA.identity.device_id,
    });
    return mapped.includes(PERSONAL_SPACE_ID) || false;
  }, TIMEOUT_MS, 1_000);

  const { labelA, labelB } = await waitForExactMatchingPersonalSyncLabels(
    actorA,
    syncedB.identity.device_id,
    actorB,
    syncedA.identity.device_id,
  );

  expect(labelA).toContain('last synced:');
  expect(labelA).toBe(labelB);
});

async function openDeviceDetailsFromSettings(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<void> {
  await actor.page.click('nav.nav a[href="#/settings"]');
  await actor.page.waitForSelector('[data-testid="settings-devices"]', TIMEOUT_MS);
  await actor.page.click(`[data-testid="paired-device-row"][data-peer-device-id="${cssString(peerDeviceId)}"] a`);
  await actor.page.waitForSelector('[data-testid="mapped-space-row"]', TIMEOUT_MS);
}

async function tickActors(actors: E2EActor[]): Promise<void> {
  for (const actor of actors) {
    await actor.invoke('space_sync_tick');
  }
}

async function waitForMatchingPersonalSyncLabel(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<string> {
  return pollUntil(`${actor.slug} Personal last synced label`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');

    const label = await actor.page.textContent(lastSyncedSelector(PERSONAL_SPACE_ID));
    const value = label?.trim() ?? '';
    return value.includes('last synced:') ? value : false;
  }, TIMEOUT_MS);
}

async function waitForExactMatchingPersonalSyncLabels(
  actorA: E2EActor,
  peerADeviceId: string,
  actorB: E2EActor,
  peerBDeviceId: string,
): Promise<{ labelA: string; labelB: string }> {
  return pollUntil('exact matching Personal last synced labels', async () => {
    await tickActors([actorA, actorB]);

    const [labelA, labelB] = await Promise.all([
      waitForMatchingPersonalSyncLabel(actorA, peerADeviceId),
      waitForMatchingPersonalSyncLabel(actorB, peerBDeviceId),
    ]);

    if (labelA !== labelB) {
      return false;
    }

    return { labelA, labelB };
  }, TIMEOUT_MS, 1_000);
}

async function waitForMappingControlsReady(actor: E2EActor, spaceId: string): Promise<void> {
  await pollUntil(`${actor.slug} mapping controls ready`, async () => {
    const state = await actor.page.evaluate<{ disabled: boolean; checked: boolean }>(`(() => {
      const checkbox = document.querySelector(${JSON.stringify(spaceCheckboxSelector(spaceId))});
      if (!(checkbox instanceof HTMLInputElement)) {
        return { disabled: true, checked: false };
      }
      return { disabled: checkbox.disabled, checked: checkbox.checked };
    })()`);

    if (state.disabled) {
      return false;
    }

    return true;
  }, TIMEOUT_MS);
}

async function waitForPeerSession(actor: E2EActor): Promise<void> {
  await pollUntil(`${actor.slug} peer session`, async () => {
    await actor.invoke('space_sync_tick');
    const debug = await actor.invoke<{ peer_session_count: number }>('device_connection_debug_status');
    return debug.peer_session_count > 0 || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForApproveDialogToClose(actor: E2EActor): Promise<void> {
  await pollUntil(`${actor.slug} approve dialog closes`, async () => {
    const state = await actor.page.evaluate<{ visible: boolean; error: string }>(`(() => {
      const dialog = document.querySelector('[data-testid="incoming-space-sync-dialog"]');
      const error = document.querySelector('[data-testid="incoming-space-sync-dialog"] .text-error')?.textContent?.trim() ?? '';
      return { visible: Boolean(dialog), error };
    })()`);

    if (state.error) {
      throw new Error(state.error);
    }

    return !state.visible || false;
  }, TIMEOUT_MS, 1_000);
}

function spaceCheckboxSelector(spaceId: string): string {
  return `${spaceRowSelector(spaceId)} [data-testid="mapped-space-checkbox"]`;
}

function lastSyncedSelector(spaceId: string): string {
  return `${spaceRowSelector(spaceId)} [data-testid="mapped-space-last-synced"]`;
}

function spaceRowSelector(spaceId: string): string {
  return `[data-testid="mapped-space-row"][data-space-id="${cssString(spaceId)}"]`;
}

function cssString(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}
