import type { E2EActor } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { openDeviceDetailsFromSettings } from './personal-sync.ts';

const DEFAULT_TIMEOUT_MS = 30_000;

export type ChannelKind = 'network' | 'bluetooth';

// The row's colour (ADR-0008 D19), decided by the backend and drawn as-is.
export type ChannelColor = 'green' | 'grey' | 'orange' | 'off' | 'none';

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
    const color = await actor.page.evaluate<string>(`(() => {
      const row = document.querySelector(${JSON.stringify(selector)});
      return row ? (row.getAttribute('data-channel-color') ?? '') : '';
    })()`);
    return color === expected ? color : false;
  }, timeoutMs, 1_000);
}

/**
 * The problem a row's ⓘ explains, or '' when it has none. The popup's text
 * is in the page whether or not it is open; only orange rows carry one.
 */
export async function channelProblem(actor: E2EActor, kind: ChannelKind): Promise<string> {
  const popupSelector = `${channelRowSelector(kind)} [data-testid="channel-problem-popup"]`;
  return actor.page.evaluate<string>(`(() => {
    const el = document.querySelector(${JSON.stringify(popupSelector)});
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
  return actor.page.evaluate<boolean>(`(() => {
    const dot = document.querySelector(${JSON.stringify(selector)});
    return dot ? dot.getAttribute('data-connected') === 'true' : false;
  })()`);
}

/** Whether any channel row on the device page is green. */
export async function anyChannelConnected(
  actor: E2EActor,
  peerDeviceId: string,
): Promise<boolean> {
  await openDeviceDetailsFromSettings(actor, peerDeviceId);
  return actor.page.evaluate<boolean>(`(() => {
    const rows = [...document.querySelectorAll('[data-testid="channel-status-row"]')];
    return rows.some((row) => row.getAttribute('data-channel-color') === 'green');
  })()`);
}

/**
 * Press Add on a channel that does not exist yet and return what the setup
 * dialog settles on: `ready` once OK is available, or the failure it shows
 * (ADR-0008 D20). Leaves the dialog open; `closeSetupDialog` ends it.
 */
export async function addChannelViaDialog(
  actor: E2EActor,
  kind: ChannelKind,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<{ ready: boolean; failure: string }> {
  await actor.page.click(`${channelRowSelector(kind)} [data-testid="add-channel"]`);
  await actor.page.waitForSelector('[data-testid="pair-device-dialog"]', timeoutMs);
  return pollUntil(`${actor.slug} ${kind} setup settles`, async () => {
    const outcome = await actor.page.evaluate<{ ready: boolean; failure: string } | null>(`(() => {
      const ok = document.querySelector('[data-testid="setup-ok"]');
      const failed = document.querySelector('[data-testid="setup-failed"]');
      if (failed) return { ready: false, failure: (failed.textContent ?? '').trim() };
      if (ok && !ok.hasAttribute('disabled')) return { ready: true, failure: '' };
      return null;
    })()`);
    return outcome ?? false;
  }, timeoutMs, 500);
}

export async function closeSetupDialog(actor: E2EActor): Promise<void> {
  await actor.page.click('[data-testid="setup-close"]');
}
