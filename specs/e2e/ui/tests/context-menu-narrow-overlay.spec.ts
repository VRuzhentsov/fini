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

async function setWindowLogicalSize(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  width: number,
  height: number,
): Promise<void> {
  await tauriPage.evaluate(`(async () => {
    const api = window.__TAURI__;
    const getCurrentWindow = api?.window?.getCurrentWindow;
    const LogicalSize = api?.dpi?.LogicalSize ?? api?.window?.LogicalSize;
    if (!getCurrentWindow || !LogicalSize) {
      throw new Error('Tauri window API unavailable (withGlobalTauri / core:window:allow-set-size?)');
    }
    await getCurrentWindow().setSize(new LogicalSize(${width}, ${height}));
  })()`);
}

test.afterEach(async ({ tauriPage }) => {
  // Restore the default test-window geometry (tauri.conf.json: 800x600) so sibling
  // specs see the wide layout. Best-effort: ignore if the API is unavailable.
  try {
    await setWindowLogicalSize(tauriPage, 800, 600);
  } catch {
    // ignore
  }
});

test('narrow window: context submenu opens as a bottom-pinned overlay above the composer', async ({ tauriPage }) => {
  const title = `e2e ctxmenu-narrow ${Date.now()}`;

  // Shrink below the 640px breakpoint so the side-by-side submenu cannot fit and
  // the in-place overlay submenu is used.
  await setWindowLogicalSize(tauriPage, 520, 720);
  await tauriPage.waitForFunction(`(() => window.innerWidth <= 640)()`, 5_000);

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

  // Right-click the quest row to open its context menu.
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

  // On a narrow window the main menu is the mobile bottom sheet; no overlay yet.
  const narrowMain = await tauriPage.evaluate<boolean>(`(() => {
    return document.querySelector('.action-sheet.mobile') !== null
      && document.querySelector('.action-sheet.overlay') === null;
  })()`);
  expect(narrowMain).toBe(true);

  // Open the "Move to space" submenu from the bottom sheet.
  await tauriPage.evaluate(`(() => {
    const hosts = Array.from(document.querySelectorAll('.action-sheet.mobile .sheet-submenu-host .sheet-item.parent'));
    const host = hosts.find((el) => el.textContent?.includes('Move to space'));
    if (!(host instanceof HTMLElement)) {
      throw new Error('"Move to space" parent row not found in mobile sheet');
    }
    host.click();
  })()`);

  await tauriPage.waitForSelector('.action-sheet.overlay', 5_000);

  const layout = await tauriPage.evaluate<{
    position: string;
    overlayTop: number;
    overlayBottom: number;
    width: number;
    height: number;
    composerTop: number;
    headTitle: string | null;
    backText: string | null;
  }>(`(() => {
    const overlay = document.querySelector('.action-sheet.overlay');
    if (!(overlay instanceof HTMLElement)) throw new Error('overlay disappeared');
    const composer = document.querySelector('.chat-composer-bar');
    const o = overlay.getBoundingClientRect();
    const c = composer instanceof HTMLElement ? composer.getBoundingClientRect() : null;
    return {
      position: getComputedStyle(overlay).position,
      overlayTop: o.top,
      overlayBottom: o.bottom,
      width: o.width,
      height: o.height,
      composerTop: c ? c.top : window.innerHeight,
      headTitle: overlay.querySelector('.sheet-overlay-title')?.textContent?.trim() ?? null,
      backText: overlay.querySelector('.sheet-back')?.textContent?.trim() ?? null,
    };
  })()`);

  expect(layout.position).toBe('fixed');
  expect(layout.width).toBeGreaterThan(0);
  expect(layout.height).toBeGreaterThan(0);
  // Pinned to the bottom inset, above the composer; never spills off the top.
  expect(layout.overlayTop).toBeGreaterThanOrEqual(0);
  expect(layout.overlayBottom).toBeLessThanOrEqual(layout.composerTop + 1);
  // Back-nav header carries the parent menu's title.
  expect(layout.headTitle).toBe('Move to space');
  expect(layout.backText).toContain('Quest actions');

  // Back returns to the mobile bottom sheet.
  await tauriPage.evaluate(`(() => {
    const back = document.querySelector('.action-sheet.overlay .sheet-back');
    if (back instanceof HTMLElement) back.click();
  })()`);
  await tauriPage.waitForFunction(
    `(() => document.querySelector('.action-sheet.overlay') === null && document.querySelector('.action-sheet.mobile') !== null)()`,
    5_000,
  );
});
