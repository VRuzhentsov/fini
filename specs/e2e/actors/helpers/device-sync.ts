import { expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { allTextContents, pollUntil, waitForText } from './dom.ts';

interface DeviceIdentity {
  device_id: string;
  hostname: string;
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

export interface SyncedActor {
  actor: E2EActor;
  identity: DeviceIdentity;
}

export interface EnsureSyncedActorsOptions {
  pairViaUi?: boolean;
  timeoutMs?: number;
  syncTicks?: number;
}

const DEFAULT_TIMEOUT_MS = 60_000;

export async function ensureSyncedActors(
  actors: E2EActor[],
  options: EnsureSyncedActorsOptions = {},
): Promise<SyncedActor[]> {
  if (actors.length < 2) {
    throw new Error(`ensureSyncedActors requires at least two actors, got ${actors.length}`);
  }

  const pairViaUi = options.pairViaUi ?? true;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const syncTicks = options.syncTicks ?? 3;
  const syncedActors = await waitForActorsReady(actors, timeoutMs);

  if (pairViaUi) {
    for (let i = 0; i < syncedActors.length; i += 1) {
      for (let j = i + 1; j < syncedActors.length; j += 1) {
        const left = syncedActors[i];
        const right = syncedActors[j];
        if (!(await areActorsPaired(left.actor, right.identity.device_id))) {
          await pairActorsViaUi(left, right, timeoutMs);
        }
      }
    }
  }

  await waitForPairedDevices(syncedActors, timeoutMs);
  await waitForPresence(syncedActors, timeoutMs);
  await driveSyncReadiness(syncedActors, syncTicks);
  await waitForPairedDeviceNames(syncedActors, timeoutMs);

  return syncedActors;
}

export async function waitForActorsReady(
  actors: E2EActor[],
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<SyncedActor[]> {
  const syncedActors: SyncedActor[] = [];

  for (const actor of actors) {
    await actor.page.waitForSelector('nav.nav', timeoutMs);
    const identity = await actor.invoke<DeviceIdentity>('device_connection_get_identity');
    expect(identity.device_id, `${actor.slug} should have a device id`).toBeTruthy();
    expect(identity.hostname, `${actor.slug} should have a hostname`).toBeTruthy();
    syncedActors.push({ actor, identity });
  }

  const ids = new Set(syncedActors.map((item) => item.identity.device_id));
  expect(ids.size, 'actor device ids should be unique').toBe(syncedActors.length);

  return syncedActors;
}

export async function pairActorsViaUi(
  requester: SyncedActor,
  accepter: SyncedActor,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<void> {
  await openAddDevice(requester.actor, timeoutMs);
  await openAddDevice(accepter.actor, timeoutMs);

  const requesterPage = requester.actor.page;
  const accepterPage = accepter.actor.page;
  const targetSelector = nearbyDeviceRequestSelector(accepter.identity.hostname);
  await requesterPage.waitForSelector(targetSelector, timeoutMs);
  await requesterPage.click(targetSelector);

  const incomingSelector = incomingRequestAcceptSelector(requester.identity.hostname);
  await accepterPage.waitForSelector(incomingSelector, timeoutMs);
  await accepterPage.click(incomingSelector);

  const code = await pollUntil(`${requester.actor.slug} outgoing pair code`, async () => {
    const updates = await requester.actor.invoke<PairCodeUpdate[]>('device_connection_pair_outgoing_updates');
    const latest = updates[0];
    if (!latest?.code) {
      return false;
    }
    return latest.code.trim();
  }, timeoutMs);
  expect(code, `pair code for ${requester.actor.slug} -> ${accepter.actor.slug}`).toMatch(/^\d{6}$/);

  await accepterPage.fill('[data-testid="pair-code-input"]', code);
  await accepterPage.click('[data-testid="pair-code-submit"]');

  await pollUntil(`${requester.actor.slug} paired with ${accepter.actor.slug}`, async () => {
    return (await areActorsPaired(requester.actor, accepter.identity.device_id)) || false;
  }, timeoutMs);
  await pollUntil(`${accepter.actor.slug} paired with ${requester.actor.slug}`, async () => {
    return (await areActorsPaired(accepter.actor, requester.identity.device_id)) || false;
  }, timeoutMs);

  await requesterPage.click('nav.nav a[href="#/settings"]');
  await accepterPage.click('nav.nav a[href="#/settings"]');
  await requesterPage.waitForSelector('[data-testid="settings-devices"]', timeoutMs);
  await accepterPage.waitForSelector('[data-testid="settings-devices"]', timeoutMs);
}

async function openAddDevice(actor: E2EActor, timeoutMs: number): Promise<void> {
  await actor.page.waitForSelector('nav.nav a[href="#/settings"]', timeoutMs);
  await actor.page.evaluate(`(() => { window.location.hash = '#/settings/add-device'; })()`);
  await actor.page.waitForSelector('[data-testid="nearby-devices"]', timeoutMs);
}

async function areActorsPaired(actor: E2EActor, peerDeviceId: string): Promise<boolean> {
  const paired = await actor.invoke<PairedDevice[]>('device_connection_get_paired_devices');
  return paired.some((device) => device.peer_device_id === peerDeviceId);
}

async function waitForPairedDevices(
  syncedActors: SyncedActor[],
  timeoutMs: number,
): Promise<void> {
  for (const current of syncedActors) {
    const peerIds = syncedActors
      .filter((candidate) => candidate.identity.device_id !== current.identity.device_id)
      .map((candidate) => candidate.identity.device_id);

    await pollUntil(`${current.actor.slug} paired devices`, async () => {
      const paired = await current.actor.invoke<PairedDevice[]>('device_connection_get_paired_devices');
      const pairedIds = new Set(paired.map((device) => device.peer_device_id));
      return peerIds.every((peerId) => pairedIds.has(peerId));
    }, timeoutMs);
  }
}

async function waitForPresence(
  syncedActors: SyncedActor[],
  timeoutMs: number,
): Promise<void> {
  for (const current of syncedActors) {
    const peerIds = syncedActors
      .filter((candidate) => candidate.identity.device_id !== current.identity.device_id)
      .map((candidate) => candidate.identity.device_id);

    await pollUntil(`${current.actor.slug} presence`, async () => {
      const present = await current.actor.invoke<DiscoveredDevice[]>('device_connection_presence_snapshot');
      const presentIds = new Set(present.map((device) => device.device_id));
      return peerIds.every((peerId) => presentIds.has(peerId));
    }, timeoutMs);
  }
}

async function driveSyncReadiness(syncedActors: SyncedActor[], syncTicks: number): Promise<void> {
  for (let tick = 0; tick < syncTicks; tick += 1) {
    for (const synced of syncedActors) {
      await synced.actor.invoke<SyncTickResult>('space_sync_tick');
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}

async function waitForPairedDeviceNames(
  syncedActors: SyncedActor[],
  timeoutMs: number,
): Promise<void> {
  for (const current of syncedActors) {
    await current.actor.page.click('nav.nav a[href="#/settings"]');
    await current.actor.page.waitForSelector('[data-testid="settings-devices"]', timeoutMs);

    for (const peer of syncedActors) {
      if (peer.identity.device_id === current.identity.device_id) {
        continue;
      }
      await waitForText(
        current.actor.page,
        '[data-testid="paired-device-name"]',
        peer.identity.hostname,
        timeoutMs,
      );
    }

    const names = await allTextContents(current.actor.page, '[data-testid="paired-device-name"]');
    for (const peer of syncedActors) {
      if (peer.identity.device_id !== current.identity.device_id) {
        expect(names, `${current.actor.slug} paired device names`).toContain(peer.identity.hostname);
      }
    }
  }
}

function nearbyDeviceRequestSelector(hostname: string): string {
  return `[data-testid="nearby-device-row"][data-device-hostname="${cssString(hostname)}"] [data-testid="request-pair"]`;
}

function incomingRequestAcceptSelector(hostname: string): string {
  return `[data-testid="incoming-request-row"][data-from-hostname="${cssString(hostname)}"] [data-testid="accept-incoming-request"]`;
}

function cssString(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}
