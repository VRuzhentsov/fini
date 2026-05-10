import { test, expect } from '../fixtures.ts';

async function fillTextarea(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  selector: string,
  value: string,
): Promise<void> {
  await tauriPage.evaluate(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    if (!(el instanceof HTMLTextAreaElement)) {
      throw new Error('textarea not found for selector: ' + ${JSON.stringify(selector)});
    }
    el.focus();
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
    if (!setter) throw new Error('textarea setter is unavailable');
    setter.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  })()`);
}

async function isSubmenuVisible(tauriPage: { evaluate: <R>(script: string) => Promise<R> }): Promise<boolean> {
  return tauriPage.evaluate<boolean>(`(() => {
    const el = document.querySelector('[data-testid="context-menu-submenu"]');
    if (!(el instanceof HTMLElement)) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  })()`);
}

test('context-menu submenu stays open during hover-to-submenu cursor traversal', async ({ tauriPage }) => {
  const title = `e2e ctxmenu-hover ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await tauriPage.evaluate(`(() => { window.location.hash = '#/quests'; })()`);
  if (!(await tauriPage.isVisible('[data-testid="chat-input"]'))) {
    await tauriPage.click('nav.nav a[href="#/main"]');
    await tauriPage.evaluate(`(() => { window.location.hash = '#/quests'; })()`);
  }
  await tauriPage.waitForSelector('[data-testid="chat-input"]', 30_000);

  await fillTextarea(tauriPage, '[data-testid="chat-input"]', title);
  await tauriPage.press('[data-testid="chat-input"]', 'Enter');

  await tauriPage.waitForSelector('.quest-row-surface', 10_000);
  await tauriPage.waitForFunction(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    return rows.some((r) => r.textContent && r.textContent.includes(${JSON.stringify(title)}));
  })()`, 10_000);

  await tauriPage.evaluate(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    const row = rows.find((candidate) => candidate.textContent?.includes(${JSON.stringify(title)}));
    if (!(row instanceof HTMLElement)) {
      throw new Error('quest row not found for title: ' + ${JSON.stringify(title)});
    }
    const rect = row.getBoundingClientRect();
    row.dispatchEvent(new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
      clientX: rect.left + rect.width / 2,
      clientY: rect.top + rect.height / 2,
    }));
  })()`);

  await tauriPage.waitForSelector('.action-sheet', 5_000);

  const wideLayout = await tauriPage.evaluate<boolean>(`(() => {
    const overlay = document.querySelector('.action-sheet.overlay');
    return overlay === null;
  })()`);
  test.skip(!wideLayout, 'submenu hover-delay only applies in wide layout (canFitSubmenuSideBySide)');

  await tauriPage.evaluate(`(() => {
    const hosts = Array.from(document.querySelectorAll('.sheet-submenu-host'));
    const host = hosts.find((el) => el.textContent?.includes('Move to space'));
    if (!(host instanceof HTMLElement)) {
      throw new Error('Move to space parent row not found in context menu');
    }
    host.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true }));
  })()`);

  await tauriPage.waitForSelector('[data-testid="context-menu-submenu"]', 5_000);
  expect(await isSubmenuVisible(tauriPage)).toBe(true);

  await tauriPage.evaluate(`(() => {
    const hosts = Array.from(document.querySelectorAll('.sheet-submenu-host'));
    const host = hosts.find((el) => el.textContent?.includes('Move to space'));
    if (!(host instanceof HTMLElement)) {
      throw new Error('Move to space parent row not found for mouseleave');
    }
    host.dispatchEvent(new MouseEvent('mouseleave', { bubbles: true }));
  })()`);

  await new Promise((resolve) => setTimeout(resolve, 150));
  expect(await isSubmenuVisible(tauriPage)).toBe(true);

  await tauriPage.evaluate(`(() => {
    const sub = document.querySelector('[data-testid="context-menu-submenu"]');
    if (!(sub instanceof HTMLElement)) {
      throw new Error('submenu element disappeared before cursor could reach it');
    }
    sub.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true }));
  })()`);

  await new Promise((resolve) => setTimeout(resolve, 400));
  expect(await isSubmenuVisible(tauriPage)).toBe(true);

  await tauriPage.evaluate(`(() => {
    const sub = document.querySelector('[data-testid="context-menu-submenu"]');
    if (!(sub instanceof HTMLElement)) return;
    sub.dispatchEvent(new MouseEvent('mouseleave', { bubbles: true }));
  })()`);

  await tauriPage.waitForFunction(`(() => {
    const el = document.querySelector('[data-testid="context-menu-submenu"]');
    if (!(el instanceof HTMLElement)) return true;
    const rect = el.getBoundingClientRect();
    return rect.width === 0 && rect.height === 0;
  })()`, 2_000);

  expect(await isSubmenuVisible(tauriPage)).toBe(false);
});
