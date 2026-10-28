import type { E2EActor } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { openDeviceDetailsFromSettings } from './personal-sync.ts';
import { ChannelColor, ChannelKind, ChannelState } from '../../../../src/utils/channel.ts';

export { ChannelColor, ChannelKind, ChannelState };

const DEFAULT_TIMEOUT_MS = 30_000;


export function channelRowSelector(kind: ChannelKind): string {
  return `[data-testid="channel-status-row"][data-channel-kind="${kind}"]`;
}

/**
 * Waits for a channel row to reach `expected`.
 *
 * Reads `data-channel-color` rather than the row's text on purpose: the
 * wording is copy and moves with the design, while the colour is the claim
 * the row makes (ADR-0008 D19).
 */
export async function waitForChannelColor(
  actor: E2EActor,
  peerDeviceId: string,
  kind: ChannelKind,
  expected: ChannelColor,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<void> {
  const selector = channelRowSelector(kind);
  await pollUntil(`${actor.slug} ${kind} row is ${expected}`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');
    const row = actor.page.locator(selector);
    const color = (await row.count()) > 0 ? await row.getAttribute('data-channel-color') : null;
    return color === expected ? color : false;
  }, timeoutMs, 1_000);
}

/**
 * The problem a row's ⓘ explains, or '' when it has none. The popup's text
 * is in the page whether or not it is open; only orange rows carry one.
 */
export async function channelProblem(actor: E2EActor, kind: ChannelKind): Promise<string> {
  const popupSelector = `${channelRowSelector(kind)} [data-testid="channel-problem-popup"]`;
  const popup = actor.page.locator(popupSelector);
  return (await popup.count()) > 0 ? ((await popup.textContent()) ?? '').trim() : '';
}

export async function toggleChannel(actor: E2EActor, kind: ChannelKind): Promise<void> {
  await actor.page.click(`${channelRowSelector(kind)} [data-testid="channel-switch"]`);
}

/** Text of the sync-queue section, whichever state it is in. */
export async function syncQueueText(actor: E2EActor): Promise<string> {
  const section = actor.page.getByTestId('sync-queue-section');
  if ((await section.count()) === 0) return '';
  return ((await section.textContent()) ?? '').replace(/\s+/g, ' ').trim();
}

export async function waitForSyncQueue(
  actor: E2EActor,
  peerDeviceId: string,
  predicate: (text: string) => boolean,
  description: string,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<string> {
  return pollUntil(`${actor.slug} sync queue ${description}`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');
    const text = await syncQueueText(actor);
    return predicate(text) ? text : false;
  }, timeoutMs, 1_000);
}

/**
 * The devices list's own verdict on a peer: whether its dot is the connected
 * colour.
 *
 * Reads `data-connected` rather than the Tailwind class, for the reason
 * `waitForChannelColor` reads `data-channel-color` -- the class is styling
 * and moves with the design, the attribute is the claim being made.
 */
export async function deviceDotConnected(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<boolean> {
  await actor.page.click('nav.nav a[href="#/settings"]');
  await actor.page.waitForSelector('[data-testid="settings-devices"]', DEFAULT_TIMEOUT_MS);
  const selector =
    `[data-testid="paired-device-row"][data-peer-device-id="${peerDeviceId}"] ` +
    `[data-testid="paired-device-dot"]`;
  const dot = actor.page.locator(selector);
  return (await dot.count()) > 0 && (await dot.getAttribute('data-connected')) === 'true';
}

/** Whether any channel row on the device page is green. */
export async function anyChannelConnected(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<boolean> {
  await openDeviceDetailsFromSettings(actor, peerDeviceId);
  return (await actor.page.locator(`[data-testid="channel-status-row"][data-channel-color="${ChannelColor.Green}"]`).count()) > 0;
}

interface ChannelStatusRow {
  kind: ChannelKind;
  state: ChannelState;
}

/**
 * The stored state of a channel with a peer, read from the backend. A
 * missing row is off (ADR-0011).
 */
export async function channelState(
  actor: E2EActor,
  peerDeviceId: string,
  kind: ChannelKind,
): Promise<ChannelState> {
  const statuses = await actor.invoke<ChannelStatusRow[]>('device_channel_statuses', {
    peerDeviceId,
  });
  return statuses.find((status) => status.kind === kind)?.state ?? ChannelState.Off;
}

export async function waitForChannelState(
  actor: E2EActor,
  peerDeviceId: string,
  kind: ChannelKind,
  expected: ChannelState,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<void> {
  await pollUntil(`${actor.slug} ${kind} channel state is ${expected}`, async () => {
    return (await channelState(actor, peerDeviceId, kind)) === expected ? expected : false;
  }, timeoutMs, 1_000);
}

