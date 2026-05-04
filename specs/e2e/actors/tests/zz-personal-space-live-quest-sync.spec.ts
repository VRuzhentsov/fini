import { test, expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  waitForPersonalLastSyncedLabel,
  waitForPersonalLastSyncedLabelChange,
} from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';

const TIMEOUT_MS = 60_000;

test('live Personal quest sync updates Focus on the peer without confirmation', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
  const questTitle = `Live sync ${Date.now()}`;

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);
  const lastSyncedBefore = await waitForPersonalLastSyncedLabel(actorB, syncedA.identity.device_id);
  await sleep(1_100);

  await openFocus(actorA);
  await submitQuest(actorA, questTitle);

  await waitForQuestOnFocus(actorA, actorB, questTitle);
  const lastSyncedAfter = await waitForPersonalLastSyncedLabelChange(
    actorB,
    syncedA.identity.device_id,
    lastSyncedBefore,
  );

  expect(lastSyncedAfter).toContain('last synced:');
  await expectNoIncomingSpaceSyncDialog(actorA);
  await expectNoIncomingSpaceSyncDialog(actorB);
});

test('second live Personal quest from peer appears in Focus backlog list', async ({ actorA, actorB }) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
  const firstQuestTitle = `Focus seed ${Date.now()}`;
  const secondQuestTitle = `Backlog peer ${Date.now()}`;

  await ensurePersonalSpaceSync(actorA, syncedB.identity.device_id, actorB, syncedA.identity.device_id);

  let activeTitle = await tryFocusActiveQuestTitle(actorA);
  if (!activeTitle) {
    await openFocus(actorA);
    await submitQuest(actorA, firstQuestTitle);
    await waitForQuestOnFocus(actorA, actorB, firstQuestTitle);
    activeTitle = await focusActiveQuestTitle(actorA);
  }

  await openFocus(actorB);
  await submitQuest(actorB, secondQuestTitle);

  await waitForQuestInBacklog(actorB, actorA, secondQuestTitle);
  expect(await focusActiveQuestTitle(actorA)).toBe(activeTitle);
});

async function openFocus(actor: E2EActor): Promise<void> {
  await actor.page.click('nav.nav a[href="#/main"]');
  await actor.page.waitForSelector('[data-testid="chat-input"]', TIMEOUT_MS);
}

async function reopenFocus(actor: E2EActor): Promise<void> {
  await actor.page.click('nav.nav a[href="#/settings"]');
  await actor.page.waitForSelector('[data-testid="settings-devices"]', TIMEOUT_MS);
  await openFocus(actor);
}

async function submitQuest(actor: E2EActor, title: string): Promise<void> {
  await actor.page.evaluate(`(() => {
    const input = document.querySelector('[data-testid="chat-input"]');
    if (!(input instanceof HTMLTextAreaElement)) {
      throw new Error('chat input textarea not found');
    }
    input.value = ${JSON.stringify(title)};
    input.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await actor.page.click('[data-testid="chat-submit"]');

  await pollUntil(`${actor.slug} local quest appears`, async () => {
    await actor.invoke('space_sync_tick');
    await openFocus(actor);
    return (await focusHasQuest(actor, title)) || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForQuestOnFocus(
  sender: E2EActor,
  receiver: E2EActor,
  title: string,
): Promise<void> {
  await openFocus(receiver);
  await pollUntil(`${receiver.slug} synced quest appears on Focus`, async () => {
    await tickActors([sender, receiver]);
    await reopenFocus(receiver);
    return (await focusHasQuest(receiver, title)) || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForQuestInBacklog(
  sender: E2EActor,
  receiver: E2EActor,
  title: string,
): Promise<void> {
  await openFocus(receiver);
  await pollUntil(`${receiver.slug} synced quest appears in backlog`, async () => {
    await tickActors([sender, receiver]);
    await reopenFocus(receiver);
    return (await backlogHasQuest(receiver, title)) || false;
  }, TIMEOUT_MS, 1_000);
}

async function focusHasQuest(actor: E2EActor, title: string): Promise<boolean> {
  return actor.page.evaluate<boolean>(`(() => {
    const titles = Array.from(document.querySelectorAll('.active-quest-title, .quest-title'));
    return titles.some((node) => node.textContent?.trim() === ${JSON.stringify(title)});
  })()`);
}

async function backlogHasQuest(actor: E2EActor, title: string): Promise<boolean> {
  return actor.page.evaluate<boolean>(`(() => {
    const titles = Array.from(document.querySelectorAll('.quest-title'));
    return titles.some((node) => node.textContent?.trim() === ${JSON.stringify(title)});
  })()`);
}

async function focusActiveQuestTitle(actor: E2EActor): Promise<string> {
  return pollUntil(`${actor.slug} active quest title`, async () => {
    await openFocus(actor);
    const title = await actor.page.textContent('.active-quest-title');
    const value = title?.trim() ?? '';
    return value || false;
  }, TIMEOUT_MS, 1_000);
}

async function tryFocusActiveQuestTitle(actor: E2EActor): Promise<string | null> {
  await openFocus(actor);
  const value = await actor.page.evaluate<string>(`(() => {
    return document.querySelector('.active-quest-title')?.textContent?.trim() ?? '';
  })()`);
  return value || null;
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function tickActors(actors: E2EActor[]): Promise<void> {
  for (const actor of actors) {
    await actor.invoke('space_sync_tick');
  }
}
