import { test, expect } from '../fixtures.ts';

type Page = {
  evaluate: <R>(script: string) => Promise<R>;
  waitForSelector: (sel: string, timeout: number) => Promise<void>;
  waitForFunction: (script: string, timeout: number) => Promise<unknown>;
  isVisible: (sel: string) => Promise<boolean>;
  click: (sel: string) => Promise<void>;
  press: (sel: string, key: string) => Promise<void>;
};

async function fillTextarea(tauriPage: Page, selector: string, value: string): Promise<void> {
  await tauriPage.evaluate(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) {
      throw new Error('text control not found for selector: ' + ${JSON.stringify(selector)});
    }
    el.focus();
    const prototype = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
    if (!setter) throw new Error('text control setter is unavailable');
    setter.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  })()`);
}

async function openQuestsView(tauriPage: Page): Promise<void> {
  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await tauriPage.evaluate(`(() => { window.location.hash = '#/quests'; })()`);
  await tauriPage.waitForSelector('[data-testid="chat-input"]', 30_000);
}

async function createQuest(tauriPage: Page, title: string): Promise<void> {
  await fillTextarea(tauriPage, '[data-testid="chat-input"]', title);
  await tauriPage.press('[data-testid="chat-input"]', 'Enter');
  await tauriPage.waitForFunction(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    return rows.some((r) => r.textContent && r.textContent.includes(${JSON.stringify(title)}));
  })()`, 10_000);
}

/** The titles of `titles` in the order of the rows on the page. */
async function rowOrder(tauriPage: Page, titles: string[]): Promise<string[]> {
  return tauriPage.evaluate<string[]>(`(() => {
    const titles = ${JSON.stringify(titles)};
    return Array.from(document.querySelectorAll('.quest-row-surface'))
      .map((row) => titles.find((title) => row.textContent?.includes(title)))
      .filter((title) => title !== undefined);
  })()`);
}

/** Drags the row of `title` with a mouse to the place of the row of `targetTitle`. */
async function dragRow(tauriPage: Page, title: string, targetTitle: string): Promise<void> {
  await tauriPage.evaluate(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    const find = (text) => rows.find((row) => row.textContent?.includes(text));
    const row = find(${JSON.stringify(title)});
    const target = find(${JSON.stringify(targetTitle)});
    if (!(row instanceof HTMLElement) || !(target instanceof HTMLElement)) {
      throw new Error('rows not found');
    }
    const start = row.getBoundingClientRect();
    const end = target.getBoundingClientRect();
    const x = start.left + start.width / 2;
    const options = { bubbles: true, cancelable: true, pointerId: 7, pointerType: 'mouse', button: 0, clientX: x };
    row.dispatchEvent(new PointerEvent('pointerdown', { ...options, clientY: start.top + start.height / 2 }));
    window.dispatchEvent(new PointerEvent('pointermove', { ...options, clientY: start.top - 20 }));
    window.dispatchEvent(new PointerEvent('pointermove', { ...options, clientY: end.top + end.height / 2 }));
    window.dispatchEvent(new PointerEvent('pointerup', { ...options, clientY: end.top + end.height / 2 }));
  })()`);
}

test('a quest dragged with the mouse gets its new place in the stored order',async ({ tauriPage }) => {
  const stamp = Date.now();
  const titles = [`e2e order one ${stamp}`, `e2e order two ${stamp}`, `e2e order three ${stamp}`];
  await openQuestsView(tauriPage);
  for (const title of titles) {
    await createQuest(tauriPage, title);
  }
  // A new quest goes to the bottom of the list.
  expect(await rowOrder(tauriPage, titles)).toEqual(titles);

  await dragRow(tauriPage, titles[2], titles[0]);
  const moved = [titles[2], titles[0], titles[1]];
  await tauriPage.waitForFunction(`(() => {
    const titles = ${JSON.stringify(moved)};
    const order = Array.from(document.querySelectorAll('.quest-row-surface'))
      .map((row) => titles.find((title) => row.textContent?.includes(title)))
      .filter((title) => title !== undefined);
    return JSON.stringify(order) === JSON.stringify(titles);
  })()`, 10_000);

  // The list shows the move before the backend answers, so wait until the
  // stored rank keys give the same order.
  await expect.poll(() => tauriPage.evaluate<string[]>(`(async () => {
    const titles = ${JSON.stringify(titles)};
    const quests = await window.__TAURI__.core.invoke('get_quests');
    return quests
      .filter((quest) => titles.includes(quest.title))
      .sort((a, b) => (a.order_rank < b.order_rank ? -1 : a.order_rank > b.order_rank ? 1 : 0))
      .map((quest) => quest.title);
  })()`), { message: 'the stored order is not the order of the drop', timeout: 10_000 }).toEqual(moved);
});
