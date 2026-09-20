import type { E2EActor } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { openDeviceDetailsFromSettings } from './personal-sync.ts';

const DEFAULT_TIMEOUT_MS = 30_000;

export type ChannelKind = 'network' | 'bluetooth';

// The row's own vocabulary (`channelRowState` in
// `src/utils/channelStatusCodes.ts`), not the backend's -- the two answer
// different questions, and this is the one the person reads.
export type ChannelRowState =
  | 'off'
  | 'waiting'
  | 'down'
  | 'connecting'
  | 'fading'
  | 'connected';

export function channelRowSelector(kind: ChannelKind): string {
  return `[data-testid="channel-status-row"][data-channel-kind="${kind}"]`;
}

/**
 * Waits for a channel row to reach `expected`.
 *
 * Reads `data-channel-state` rather than the row's text on purpose: the
 * wording is copy and moves with the design, while this attribute is the
 * state machine behind it. An earlier version of the BLE lane asserted on
 * the words and broke on a rename that changed nothing it was protecting.
 */
export async function waitForChannelState(
  actor: E2EActor,
  peerDeviceId: string,
  kind: ChannelKind,
  expected: ChannelRowState,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<void> {
  const selector = channelRowSelector(kind);
  await pollUntil(`${actor.slug} ${kind} row is "${expected}"`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');
    const state = await actor.page.evaluate<string>(`(() => {
      const row = document.querySelector(${JSON.stringify(selector)});
      return row ? (row.getAttribute('data-channel-state') ?? '') : '';
    })()`);
    return state === expected ? state : false;
  }, timeoutMs, 1_000);
}

/** The plain-language reason under a row, or '' when it isn't showing one. */
export async function channelReason(actor: E2EActor, kind: ChannelKind): Promise<string> {
  const selector = `${channelRowSelector(kind)} [data-testid="channel-status-reason"]`;
  return actor.page.evaluate<string>(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    return el ? (el.textContent ?? '').trim() : '';
  })()`);
}

export async function toggleChannel(actor: E2EActor, kind: ChannelKind): Promise<void> {
  await actor.page.click(`${channelRowSelector(kind)} [data-testid="channel-switch"]`);
}

/** Text of the sync-queue section, whichever state it is in. */
export async function syncQueueText(actor: E2EActor): Promise<string> {
  return actor.page.evaluate<string>(`(() => {
    const el = document.querySelector('[data-testid="sync-queue-section"]');
    return el ? (el.textContent ?? '').replace(/\\s+/g, ' ').trim() : '';
  })()`);
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
