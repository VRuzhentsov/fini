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

/**
 * The plain-language reason for a row, asked for the way a person asks for
 * it: by pressing the row's information button.
 *
 * It clicks rather than just reading, because no row expands its reason on
 * its own any more -- reading the page without asking would return '' for
 * every state. Only clicks when the reason is not already open, so calling
 * this twice does not toggle it shut again.
 *
 * Returns '' when the row has no reason at all, which is a connected row.
 */
export async function channelReason(actor: E2EActor, kind: ChannelKind): Promise<string> {
  const rowSelector = channelRowSelector(kind);
  const reasonSelector = `${rowSelector} [data-testid="channel-status-reason"]`;
  const infoSelector = `${rowSelector} [data-testid="channel-status-info"]`;

  const needsOpening = await actor.page.evaluate<boolean>(`(() => {
    const reason = document.querySelector(${JSON.stringify(reasonSelector)});
    const info = document.querySelector(${JSON.stringify(infoSelector)});
    return !reason && !!info;
  })()`);
  if (needsOpening) {
    await actor.page.click(infoSelector);
  }

  return actor.page.evaluate<string>(`(() => {
    const el = document.querySelector(${JSON.stringify(reasonSelector)});
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

/**
 * The devices list's own verdict on a peer: whether its dot is the connected
 * colour.
 *
 * Reads `data-connected` rather than the Tailwind class, for the reason
 * `waitForChannelState` reads `data-channel-state` -- the class is styling
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
  return actor.page.evaluate<boolean>(`(() => {
    const dot = document.querySelector(${JSON.stringify(selector)});
    return dot ? dot.getAttribute('data-connected') === 'true' : false;
  })()`);
}

/** Whether any channel row on the device page is in the connected state. */
export async function anyChannelConnected(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<boolean> {
  await openDeviceDetailsFromSettings(actor, peerDeviceId);
  return actor.page.evaluate<boolean>(`(() => {
    const rows = [...document.querySelectorAll('[data-testid="channel-status-row"]')];
    return rows.some((row) => row.getAttribute('data-channel-state') === 'connected');
  })()`);
}

/**
 * Switch a channel on that was never set up, which now opens the setup
 * dialog rather than writing anything: search for the peer, then accept the
 * outcome either way.
 *
 * "Turn on anyway" is the branch this takes in the container, where there is
 * no `bluetoothd` and the search cannot succeed -- and it is the branch a
 * person with the radio switched off takes too. The switch alone used to do
 * this; the dialog owns it now, so the test has to walk the same path.
 */
export async function setUpChannelViaDialog(
  actor: E2EActor,
  kind: ChannelKind,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<void> {
  await actor.page.click(`${channelRowSelector(kind)} [data-testid="channel-switch"]`);
  await actor.page.waitForSelector('[data-testid="channel-setup-dialog"]', timeoutMs);
  await actor.page.click(`[data-testid="setup-${kind}"]`);

  // Either the search found it or it did not; both end somewhere with a way
  // forward, and the dialog is what decides which.
  await pollUntil(`${actor.slug} ${kind} setup offers a way to turn it on`, async () => {
    return actor.page.evaluate<boolean>(`(() => {
      return !!document.querySelector('[data-testid="turn-on-${kind}"], [data-testid="turn-on-${kind}-anyway"]');
    })()`);
  }, timeoutMs, 500);

  const found = await actor.page.evaluate<boolean>(`(() => {
    return !!document.querySelector('[data-testid="turn-on-${kind}"]');
  })()`);
  await actor.page.click(
    found ? `[data-testid="turn-on-${kind}"]` : `[data-testid="turn-on-${kind}-anyway"]`,
  );

  await actor.page.click('[data-testid="channel-setup-backdrop"]');
}
