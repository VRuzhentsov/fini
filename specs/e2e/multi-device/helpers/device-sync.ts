import { expect } from '../test-device.ts';
import type { TestDevice } from '../test-device.ts';
import { allTextContents, pollUntil, waitForText } from './dom.ts';

interface DeviceIdentity {
  device_id: string;
  hostname: string;
  endpoint_id: string;
}

interface PairedDevice {
  peer_device_id: string;
  display_name: string;
}

interface DiscoveredDevice {
  device_id: string;
  hostname: string;
}

interface SyncTickResult {
  sent_events: number;
  applied_events: number;
  received_acks: number;
}

interface PairCodeUpdate {
  request_id: string;
  code: string;
  accepted_at: string;
}

export interface SyncedTestDevice {
  testDevice: TestDevice;
  identity: DeviceIdentity;
}

export interface EnsureSyncedTestDevicesOptions {
  pairViaUi?: boolean;
  timeoutMs?: number;
  syncTicks?: number;
}

const DEFAULT_TIMEOUT_MS = 60_000;

export async function ensureSyncedTestDevices(
  testDevices: TestDevice[],
  options: EnsureSyncedTestDevicesOptions = {},
): Promise<SyncedTestDevice[]> {
  if (testDevices.length < 2) {
    throw new Error(`ensureSyncedTestDevices requires at least two testDevices, got ${testDevices.length}`);
  }

  const pairViaUi = options.pairViaUi ?? true;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const syncTicks = options.syncTicks ?? 3;
  const syncedTestDevices = await waitForTestDevicesReady(testDevices, timeoutMs);

  if (pairViaUi) {
    for (let i = 0; i < syncedTestDevices.length; i += 1) {
      for (let j = i + 1; j < syncedTestDevices.length; j += 1) {
        const left = syncedTestDevices[i];
        const right = syncedTestDevices[j];
        if (!(await areTestDevicesPaired(left.testDevice, right.identity.device_id))) {
          await pairTestDevicesViaUi(left, right, timeoutMs);
        }
      }
    }
  }

  await waitForPairedDevices(syncedTestDevices, timeoutMs);
  await waitForPresence(syncedTestDevices, timeoutMs);
  await driveSyncReadiness(syncedTestDevices, syncTicks);
  await waitForPairedDeviceNames(syncedTestDevices, timeoutMs);

  return syncedTestDevices;
}

export async function waitForTestDevicesReady(
  testDevices: TestDevice[],
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<SyncedTestDevice[]> {
  const syncedTestDevices: SyncedTestDevice[] = [];

  for (const testDevice of testDevices) {
    await testDevice.page.waitForSelector('nav.nav', timeoutMs);
    const identity = await testDevice.invoke<DeviceIdentity>('device_get_identity');
    expect(identity.device_id, `${testDevice.slug} should have a device id`).toBeTruthy();
    expect(identity.hostname, `${testDevice.slug} should have a hostname`).toBeTruthy();
    syncedTestDevices.push({ testDevice, identity });
  }

  const ids = new Set(syncedTestDevices.map((item) => item.identity.device_id));
  expect(ids.size, 'testDevice device ids should be unique').toBe(syncedTestDevices.length);

  return syncedTestDevices;
}

export async function pairTestDevicesViaUi(
  requester: SyncedTestDevice,
  accepter: SyncedTestDevice,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<void> {
  await openAddDevice(requester.testDevice, timeoutMs);
  await openAddDevice(accepter.testDevice, timeoutMs);

  const requesterPage = requester.testDevice.page;
  const accepterPage = accepter.testDevice.page;

  // The channel is an explicit choice now, asked before discovery runs --
  // it is no longer inferred from whichever radio found the peer first.
  // Both sides pick Network: the testDevices harness has no radio. A device is
  // in Add Device mode, and so can be found, only once its channel is chosen.
  await requesterPage.click('[data-testid="pair-channel-network"]');
  await accepterPage.click('[data-testid="pair-channel-network"]');

  const targetSelector = nearbyDeviceRequestSelector(accepter.identity.hostname);
  await requesterPage.waitForSelector(targetSelector, timeoutMs);
  await requesterPage.click(targetSelector);

  // The accepter's dialog switches itself to the incoming request the
  // moment one arrives, whichever step it was showing -- someone waiting on
  // an answer outranks whatever this side was doing.
  const incomingSelector = incomingRequestAcceptSelector();
  await accepterPage.waitForSelector(incomingSelector, timeoutMs);
  await accepterPage.click(incomingSelector);

  const code = await pollUntil(`${requester.testDevice.slug} outgoing pair code`, async () => {
    const updates = await requester.testDevice.invoke<PairCodeUpdate[]>('device_pair_outgoing_updates');
    const latest = updates[0];
    if (!latest?.code) {
      return false;
    }
    return latest.code.trim();
  }, timeoutMs);
  expect(code, `pair code for ${requester.testDevice.slug} -> ${accepter.testDevice.slug}`).toMatch(/^\d{6}$/);

  await accepterPage.fill('[data-testid="pair-code-input"]', code);
  await accepterPage.click('[data-testid="pair-code-submit"]');

  await pollUntil(`${requester.testDevice.slug} paired with ${accepter.testDevice.slug}`, async () => {
    return (await areTestDevicesPaired(requester.testDevice, accepter.identity.device_id)) || false;
  }, timeoutMs);
  await pollUntil(`${accepter.testDevice.slug} paired with ${requester.testDevice.slug}`, async () => {
    return (await areTestDevicesPaired(accepter.testDevice, requester.identity.device_id)) || false;
  }, timeoutMs);

  // Dismiss the dialog on both sides before leaving: it stays mounted with
  // the Settings page behind it, and leaving it open would also leave both
  // devices sitting in add mode.
  await closePairDialog(requesterPage, timeoutMs);
  await closePairDialog(accepterPage, timeoutMs);

  await requesterPage.click('nav.nav a[href="#/settings"]');
  await accepterPage.click('nav.nav a[href="#/settings"]');
  await requesterPage.waitForSelector('[data-testid="settings-devices"]', timeoutMs);
  await accepterPage.waitForSelector('[data-testid="settings-devices"]', timeoutMs);
}

// Closes the pairing dialog by its backdrop, which is present in every step
// -- the footer's own dismiss changes label ("Cancel", "Close", "Done") as
// the flow progresses, so keying off it would make this depend on which
// step pairing happened to end on.
async function closePairDialog(page: TestDevice['page'], timeoutMs: number): Promise<void> {
  const selector = '[data-testid="pair-dialog-backdrop"]';
  await page.waitForSelector(selector, timeoutMs);
  await page.click(selector);
}

// Adding a device is a dialog on the Settings page rather than its own page,
// and opening it is what puts this device into add mode -- which is what
// makes it discoverable to the other one.
//
// Driven by clicking the row a person clicks. There is no route to navigate
// to any more, and that is the point: the only way in is the one the product
// actually offers, so this cannot pass while the button is broken.
async function openAddDevice(testDevice: TestDevice, timeoutMs: number): Promise<void> {
  await testDevice.page.waitForSelector('nav.nav a[href="#/settings"]', timeoutMs);
  await testDevice.page.click('nav.nav a[href="#/settings"]');
  await testDevice.page.waitForSelector('[data-testid="add-device-link"]', timeoutMs);
  await testDevice.page.click('[data-testid="add-device-link"]');
  await testDevice.page.waitForSelector('[data-testid="pair-device-dialog"]', timeoutMs);
}

async function areTestDevicesPaired(testDevice: TestDevice, peerDeviceId: string): Promise<boolean> {
  const paired = await testDevice.invoke<PairedDevice[]>('device_paired_devices');
  return paired.some((device) => device.peer_device_id === peerDeviceId);
}

async function waitForPairedDevices(
  syncedTestDevices: SyncedTestDevice[],
  timeoutMs: number,
): Promise<void> {
  for (const current of syncedTestDevices) {
    const peerIds = syncedTestDevices
      .filter((candidate) => candidate.identity.device_id !== current.identity.device_id)
      .map((candidate) => candidate.identity.device_id);

    await pollUntil(`${current.testDevice.slug} paired devices`, async () => {
      const paired = await current.testDevice.invoke<PairedDevice[]>('device_paired_devices');
      const pairedIds = new Set(paired.map((device) => device.peer_device_id));
      return peerIds.every((peerId) => pairedIds.has(peerId));
    }, timeoutMs);
  }
}

async function waitForPresence(
  syncedTestDevices: SyncedTestDevice[],
  timeoutMs: number,
): Promise<void> {
  for (const current of syncedTestDevices) {
    const peerIds = syncedTestDevices
      .filter((candidate) => candidate.identity.device_id !== current.identity.device_id)
      .map((candidate) => candidate.identity.device_id);

    await pollUntil(`${current.testDevice.slug} presence`, async () => {
      const present = await current.testDevice.invoke<DiscoveredDevice[]>('device_heard_devices');
      const presentIds = new Set(present.map((device) => device.device_id));
      return peerIds.every((peerId) => presentIds.has(peerId));
    }, timeoutMs);
  }
}

async function driveSyncReadiness(syncedTestDevices: SyncedTestDevice[], syncTicks: number): Promise<void> {
  for (let tick = 0; tick < syncTicks; tick += 1) {
    for (const synced of syncedTestDevices) {
      await synced.testDevice.invoke<SyncTickResult>('space_sync_tick');
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}

async function waitForPairedDeviceNames(
  syncedTestDevices: SyncedTestDevice[],
  timeoutMs: number,
): Promise<void> {
  for (const current of syncedTestDevices) {
    await current.testDevice.page.click('nav.nav a[href="#/settings"]');
    await current.testDevice.page.waitForSelector('[data-testid="settings-devices"]', timeoutMs);

    for (const peer of syncedTestDevices) {
      if (peer.identity.device_id === current.identity.device_id) {
        continue;
      }
      await waitForText(
        current.testDevice.page,
        '[data-testid="paired-device-name"]',
        peer.identity.hostname,
        timeoutMs,
      );
    }

    const names = await allTextContents(current.testDevice.page, '[data-testid="paired-device-name"]');
    for (const peer of syncedTestDevices) {
      if (peer.identity.device_id !== current.identity.device_id) {
        expect(names, `${current.testDevice.slug} paired device names`).toContain(peer.identity.hostname);
      }
    }
  }
}

function nearbyDeviceRequestSelector(hostname: string): string {
  return `[data-testid="nearby-device-row"][data-device-hostname="${cssString(hostname)}"] [data-testid="request-pair"]`;
}

// Accepting happens inside the pairing dialog, which shows one request at a
// time -- so this no longer keys off the sender's hostname the way the old
// per-row Accept button did.
function incomingRequestAcceptSelector(): string {
  return '[data-testid="pair-device-dialog"] [data-testid="accept-incoming-request"]';
}

function cssString(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}
