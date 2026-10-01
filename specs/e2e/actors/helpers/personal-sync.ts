import type { E2EActor } from '../fixtures.ts';
import { expect } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

const PERSONAL_SPACE_ID = '1';
const TIMEOUT_MS = 60_000;

export async function ensurePersonalSpaceSync(
  requester: E2EActor,
  requesterPeerDeviceId: string,
  approver: E2EActor,
  approverPeerDeviceId: string,
  kind: ChannelKind = ChannelKind.Network,
): Promise<void> {
  // Presence, not a session: an idle pair has no session (ADR-0008 D10).
  // Saving the mapping below is the work that opens the exchange.
  await waitForChannelGreen(requester, requesterPeerDeviceId, kind);
  await waitForChannelGreen(approver, approverPeerDeviceId, kind);

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

export async function expectNoIncomingSpaceSyncDialog(actor: E2EActor): Promise<void> {
  const count = await actor.page.getByTestId('incoming-space-sync-dialog').count();
  expect(count, `${actor.slug} should not show incoming space sync dialog`).toBe(0);
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

export async function waitForMappingControlsReady(actor: E2EActor, spaceId: string): Promise<void> {
  await pollUntil(`${actor.slug} mapping controls ready`, async () => {
    const checkbox = actor.page.locator(spaceCheckboxSelector(spaceId));
    if ((await checkbox.count()) === 0) {
      return false;
    }
    return !(await checkbox.isDisabled());
  }, TIMEOUT_MS);
}

interface ChannelStatusRow {
  kind: ChannelKind;
  color: ChannelColor;
}

/**
 * Waits until the backend reports `kind` green for this peer: the peer was
 * heard on that channel within its timeout (ADR-0008 D9).
 */
export async function waitForChannelGreen(
  actor: E2EActor,
  peerDeviceId: string,
  kind: ChannelKind,
): Promise<void> {
  await pollUntil(`${actor.slug} ${kind} channel green`, async () => {
    await actor.invoke('space_sync_tick');
    const statuses = await actor.invoke<ChannelStatusRow[]>('device_connection_channel_statuses', {
      peerDeviceId,
    });
    return statuses.some((status) => status.kind === kind && status.color === ChannelColor.Green) || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForApproveDialogToClose(actor: E2EActor): Promise<void> {
  await pollUntil(`${actor.slug} approve dialog closes`, async () => {
    const dialog = actor.page.getByTestId('incoming-space-sync-dialog');
    if ((await dialog.count()) === 0) {
      return true;
    }
    const errorText = dialog.locator('.text-error');
    const error = (await errorText.count()) > 0 ? ((await errorText.textContent())?.trim() ?? '') : '';
    if (error) {
      throw new Error(error);
    }
    return false;
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
