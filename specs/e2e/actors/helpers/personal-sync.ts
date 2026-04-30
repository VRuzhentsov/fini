import type { E2EActor } from '../fixtures.js';
import { expect } from '../fixtures.js';
import { pollUntil } from './dom.js';

const PERSONAL_SPACE_ID = '1';
const TIMEOUT_MS = 60_000;

export async function ensurePersonalSpaceSync(
  requester: E2EActor,
  requesterPeerDeviceId: string,
  approver: E2EActor,
  approverPeerDeviceId: string,
): Promise<void> {
  await waitForPeerSession(requester);
  await waitForPeerSession(approver);

  const mapped = await requester.invoke<string[]>('space_sync_list_mappings', {
    peerDeviceId: requesterPeerDeviceId,
  });

  if (!mapped.includes(PERSONAL_SPACE_ID)) {
    await openDeviceDetailsFromSettings(requester, requesterPeerDeviceId);
    await waitForMappingControlsReady(requester, PERSONAL_SPACE_ID);
    await requester.page.click(spaceCheckboxSelector(PERSONAL_SPACE_ID));
    await requester.page.waitForSelector('[data-testid="save-space-mappings"]:not([disabled])', TIMEOUT_MS);
    await requester.page.click('[data-testid="save-space-mappings"]');

    await pollUntil('A persists Personal mapping', async () => {
      const nextMapped = await requester.invoke<string[]>('space_sync_list_mappings', {
        peerDeviceId: requesterPeerDeviceId,
      });
      return nextMapped.includes(PERSONAL_SPACE_ID) || false;
    }, TIMEOUT_MS);

    await approver.page.waitForSelector(
      `[data-testid="incoming-space-sync-dialog"][data-dialog-kind="approve"][data-peer-device-id="${cssString(approverPeerDeviceId)}"][data-space-count="1"]`,
      TIMEOUT_MS,
    );
    await approver.page.click('[data-testid="approve-space-sync"]');
    await waitForApproveDialogToClose(approver);

    await pollUntil('B persists approved Personal mapping', async () => {
      await tickActors([requester, approver]);
      const nextMapped = await approver.invoke<string[]>('space_sync_list_mappings', {
        peerDeviceId: approverPeerDeviceId,
      });
      return nextMapped.includes(PERSONAL_SPACE_ID) || false;
    }, TIMEOUT_MS, 1_000);
  }
}

export async function openDeviceDetailsFromSettings(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<void> {
  await actor.page.click('nav.nav a[href="#/settings"]');
  await actor.page.waitForSelector('[data-testid="settings-devices"]', TIMEOUT_MS);
  await actor.page.click(`[data-testid="paired-device-row"][data-peer-device-id="${cssString(peerDeviceId)}"] a`);
  await actor.page.waitForSelector('[data-testid="mapped-space-row"]', TIMEOUT_MS);
}

export async function waitForPersonalLastSyncedLabel(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<string> {
  return pollUntil(`${actor.slug} Personal last synced label`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');
    const label = await actor.page.textContent(lastSyncedSelector(PERSONAL_SPACE_ID));
    const value = label?.trim() ?? '';
    return value.includes('last synced:') ? value : false;
  }, TIMEOUT_MS, 1_000);
}

export async function waitForPersonalLastSyncedLabelChange(
  actor: E2EActor,
  peerDeviceId: string,
  previousLabel: string,
): Promise<string> {
  return pollUntil(`${actor.slug} Personal last synced label update`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');
    const label = await actor.page.textContent(lastSyncedSelector(PERSONAL_SPACE_ID));
    const value = label?.trim() ?? '';
    if (!value.includes('last synced:')) {
      return false;
    }
    return value !== previousLabel ? value : false;
  }, TIMEOUT_MS, 1_000);
}

async function tickActors(actors: E2EActor[]): Promise<void> {
  for (const actor of actors) {
    await actor.invoke('space_sync_tick');
  }
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
