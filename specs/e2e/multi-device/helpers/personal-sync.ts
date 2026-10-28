import type { TestDevice } from '../test-device.ts';
import { expect } from '../test-device.ts';
import { pollUntil } from './dom.ts';
import { ChannelColor, ChannelKind } from '../../../../src/utils/channel.ts';

const PERSONAL_SPACE_ID = '1';
const TIMEOUT_MS = 60_000;

export async function ensurePersonalSpaceSync(
  requester: TestDevice,
  requesterPeerDeviceId: string,
  approver: TestDevice,
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
      await tickTestDevices([requester, approver]);
      const nextMapped = await approver.invoke<string[]>('space_sync_list_mappings', {
        peerDeviceId: approverPeerDeviceId,
      });
      return nextMapped.includes(PERSONAL_SPACE_ID) || false;
    }, TIMEOUT_MS, 1_000);
  }
}

export async function expectNoIncomingSpaceSyncDialog(testDevice: TestDevice): Promise<void> {
  const count = await testDevice.page.getByTestId('incoming-space-sync-dialog').count();
  expect(count, `${testDevice.slug} should not show incoming space sync dialog`).toBe(0);
}

export async function openDeviceDetailsFromSettings(
  testDevice: TestDevice,
  peerDeviceId: string,
): Promise<void> {
  await testDevice.page.click('nav.nav a[href="#/settings"]');
  await testDevice.page.waitForSelector('[data-testid="settings-devices"]', TIMEOUT_MS);
  await testDevice.page.click(`[data-testid="paired-device-row"][data-peer-device-id="${cssString(peerDeviceId)}"] a`);
  await testDevice.page.waitForSelector('[data-testid="mapped-space-row"]', TIMEOUT_MS);
}

export async function waitForPersonalLastSyncedLabel(
  testDevice: TestDevice,
  peerDeviceId: string,
): Promise<string> {
  return pollUntil(`${testDevice.slug} Personal last synced label`, async () => {
    await openDeviceDetailsFromSettings(testDevice, peerDeviceId);
    await testDevice.invoke('space_sync_tick');
    const label = await testDevice.page.textContent(lastSyncedSelector(PERSONAL_SPACE_ID));
    const value = label?.trim() ?? '';
    return value.includes('last synced:') ? value : false;
  }, TIMEOUT_MS, 1_000);
}

export async function waitForPersonalLastSyncedLabelChange(
  testDevice: TestDevice,
  peerDeviceId: string,
  previousLabel: string,
): Promise<string> {
  return pollUntil(`${testDevice.slug} Personal last synced label update`, async () => {
    await openDeviceDetailsFromSettings(testDevice, peerDeviceId);
    await testDevice.invoke('space_sync_tick');
    const label = await testDevice.page.textContent(lastSyncedSelector(PERSONAL_SPACE_ID));
    const value = label?.trim() ?? '';
    if (!value.includes('last synced:')) {
      return false;
    }
    return value !== previousLabel ? value : false;
  }, TIMEOUT_MS, 1_000);
}

async function tickTestDevices(testDevices: TestDevice[]): Promise<void> {
  for (const testDevice of testDevices) {
    await testDevice.invoke('space_sync_tick');
  }
}

export async function waitForMappingControlsReady(testDevice: TestDevice, spaceId: string): Promise<void> {
  await pollUntil(`${testDevice.slug} mapping controls ready`, async () => {
    const checkbox = testDevice.page.locator(spaceCheckboxSelector(spaceId));
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
  testDevice: TestDevice,
  peerDeviceId: string,
  kind: ChannelKind,
): Promise<void> {
  await pollUntil(`${testDevice.slug} ${kind} channel green`, async () => {
    await testDevice.invoke('space_sync_tick');
    const statuses = await testDevice.invoke<ChannelStatusRow[]>('device_channel_statuses', {
      peerDeviceId,
    });
    return statuses.some((status) => status.kind === kind && status.color === ChannelColor.Green) || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForApproveDialogToClose(testDevice: TestDevice): Promise<void> {
  await pollUntil(`${testDevice.slug} approve dialog closes`, async () => {
    const dialog = testDevice.page.getByTestId('incoming-space-sync-dialog');
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
