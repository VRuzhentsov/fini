import { test, expect } from '../test-device.ts';
import type { TestDevice } from '../test-device.ts';
import { ensureSyncedTestDevices } from '../helpers/device-sync.ts';
import {
  ensurePersonalSpaceSync,
  expectNoIncomingSpaceSyncDialog,
  waitForPersonalLastSyncedLabel,
  waitForPersonalLastSyncedLabelChange,
} from '../helpers/personal-sync.ts';
import { pollUntil } from '../helpers/dom.ts';

const TIMEOUT_MS = 60_000;

test('live Personal quest sync updates Focus on the peer without confirmation', async ({ testDeviceA, testDeviceB }) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
  const questTitle = `Live sync ${Date.now()}`;

  await ensurePersonalSpaceSync(testDeviceA, syncedB.identity.device_id, testDeviceB, syncedA.identity.device_id);
  const lastSyncedBefore = await waitForPersonalLastSyncedLabel(testDeviceB, syncedA.identity.device_id);
  await sleep(1_100);

  await openFocus(testDeviceA);
  await submitQuest(testDeviceA, questTitle);

  await waitForQuestOnFocus(testDeviceA, testDeviceB, questTitle);
  const lastSyncedAfter = await waitForPersonalLastSyncedLabelChange(
    testDeviceB,
    syncedA.identity.device_id,
    lastSyncedBefore,
  );

  expect(lastSyncedAfter).toContain('last synced:');
  await expectNoIncomingSpaceSyncDialog(testDeviceA);
  await expectNoIncomingSpaceSyncDialog(testDeviceB);
});

test('second live Personal quest from peer appears in Focus backlog list', async ({ testDeviceA, testDeviceB }) => {
  const [syncedA, syncedB] = await ensureSyncedTestDevices([testDeviceA, testDeviceB], { pairViaUi: true });
  const firstQuestTitle = `Focus seed ${Date.now()}`;
  const secondQuestTitle = `Backlog peer ${Date.now()}`;

  await ensurePersonalSpaceSync(testDeviceA, syncedB.identity.device_id, testDeviceB, syncedA.identity.device_id);

  let activeTitle = await tryFocusActiveQuestTitle(testDeviceA);
  if (!activeTitle) {
    await openFocus(testDeviceA);
    await submitQuest(testDeviceA, firstQuestTitle);
    await waitForQuestOnFocus(testDeviceA, testDeviceB, firstQuestTitle);
    activeTitle = await focusActiveQuestTitle(testDeviceA);
  }

  await openFocus(testDeviceB);
  await submitQuest(testDeviceB, secondQuestTitle);

  await waitForQuestInBacklog(testDeviceB, testDeviceA, secondQuestTitle);
  expect(await focusActiveQuestTitle(testDeviceA)).toBe(activeTitle);
});

async function openFocus(testDevice: TestDevice): Promise<void> {
  await testDevice.page.click('nav.nav a[href="#/main"]');
  await testDevice.page.waitForSelector('[data-testid="chat-input"]', TIMEOUT_MS);
}

async function reopenFocus(testDevice: TestDevice): Promise<void> {
  await testDevice.page.click('nav.nav a[href="#/settings"]');
  await testDevice.page.waitForSelector('[data-testid="settings-devices"]', TIMEOUT_MS);
  await openFocus(testDevice);
}

async function submitQuest(testDevice: TestDevice, title: string): Promise<void> {
  await testDevice.page.evaluate(`(() => {
    const input = document.querySelector('[data-testid="chat-input"]');
    if (!(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)) {
      throw new Error('chat input text control not found');
    }
    const prototype = input instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
    if (!setter) throw new Error('chat input setter is unavailable');
    setter.call(input, ${JSON.stringify(title)});
    input.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await testDevice.page.click('[data-testid="chat-submit"]');

  await pollUntil(`${testDevice.slug} local quest appears`, async () => {
    await testDevice.invoke('space_sync_tick');
    await openFocus(testDevice);
    return (await focusHasQuest(testDevice, title)) || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForQuestOnFocus(
  sender: TestDevice,
  receiver: TestDevice,
  title: string,
): Promise<void> {
  await openFocus(receiver);
  await pollUntil(`${receiver.slug} synced quest appears on Focus`, async () => {
    await tickTestDevices([sender, receiver]);
    await reopenFocus(receiver);
    return (await focusHasQuest(receiver, title)) || false;
  }, TIMEOUT_MS, 1_000);
}

async function waitForQuestInBacklog(
  sender: TestDevice,
  receiver: TestDevice,
  title: string,
): Promise<void> {
  await openFocus(receiver);
  await pollUntil(`${receiver.slug} synced quest appears in backlog`, async () => {
    await tickTestDevices([sender, receiver]);
    await reopenFocus(receiver);
    return (await backlogHasQuest(receiver, title)) || false;
  }, TIMEOUT_MS, 1_000);
}

async function focusHasQuest(testDevice: TestDevice, title: string): Promise<boolean> {
  return testDevice.page.evaluate<boolean>(`(() => {
    const titles = Array.from(document.querySelectorAll('.active-quest-title, .quest-title'));
    return titles.some((node) => node.textContent?.trim() === ${JSON.stringify(title)});
  })()`);
}

async function backlogHasQuest(testDevice: TestDevice, title: string): Promise<boolean> {
  return testDevice.page.evaluate<boolean>(`(() => {
    const titles = Array.from(document.querySelectorAll('.quest-title'));
    return titles.some((node) => node.textContent?.trim() === ${JSON.stringify(title)});
  })()`);
}

async function focusActiveQuestTitle(testDevice: TestDevice): Promise<string> {
  return pollUntil(`${testDevice.slug} active quest title`, async () => {
    await openFocus(testDevice);
    const title = await testDevice.page.textContent('.active-quest-title');
    const value = title?.trim() ?? '';
    return value || false;
  }, TIMEOUT_MS, 1_000);
}

async function tryFocusActiveQuestTitle(testDevice: TestDevice): Promise<string | null> {
  await openFocus(testDevice);
  const value = await testDevice.page.evaluate<string>(`(() => {
    return document.querySelector('.active-quest-title')?.textContent?.trim() ?? '';
  })()`);
  return value || null;
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function tickTestDevices(testDevices: TestDevice[]): Promise<void> {
  for (const testDevice of testDevices) {
    await testDevice.invoke('space_sync_tick');
  }
}
