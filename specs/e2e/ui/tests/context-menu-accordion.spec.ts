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
      throw new Error('Tauri window API unavailable');
    }
    await getCurrentWindow().setSize(new LogicalSize(${width}, ${height}));
  })()`);
}

async function openContextMenuForQuest(
  tauriPage: { evaluate: <R>(script: string) => Promise<R>; waitForSelector: (sel: string, timeout: number) => Promise<void>; isVisible: (sel: string) => Promise<boolean>; click: (sel: string) => Promise<void>; press: (sel: string, key: string) => Promise<void> },
  title: string,
): Promise<void> {
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
}

test.afterEach(async ({ tauriPage }) => {
  // Close any open context menu so it doesn't leak into the next spec
  try {
    await tauriPage.press('body', 'Escape');
  } catch {
    // ignore
  }
  try {
    await setWindowLogicalSize(tauriPage, 800, 600);
  } catch {
    // ignore
  }
});

test('wide: Move to space opens as in-place accordion, not a flyout', async ({ tauriPage }) => {
  const title = `e2e ctx-accordion-wide ${Date.now()}`;
  await openContextMenuForQuest(tauriPage, title);

  // Click the "Move to space" parent row
  await tauriPage.evaluate(`(() => {
    const btn = Array.from(document.querySelectorAll('[data-testid="context-menu"] .sheet-item[aria-expanded]'))
      .find((el) => el.textContent?.includes('Move to space'));
    if (!(btn instanceof HTMLElement)) throw new Error('"Move to space" row not found');
    btn.click();
  })()`);

  await tauriPage.waitForFunction(`(() => {
    return document.querySelectorAll('[data-testid="context-menu"] .sheet-item.child').length > 0;
  })()`, 3_000);

  const result = await tauriPage.evaluate<{
    parentExpanded: string | null;
    childCount: number;
    noFlyout: boolean;
    noOverlay: boolean;
  }>(`(() => {
    const parent = Array.from(document.querySelectorAll('[data-testid="context-menu"] .sheet-item[aria-expanded]'))
      .find((el) => el.textContent?.includes('Move to space'));
    return {
      parentExpanded: parent ? parent.getAttribute('aria-expanded') : null,
      childCount: document.querySelectorAll('[data-testid="context-menu"] .sheet-item.child').length,
      noFlyout:  document.querySelector('[data-testid="context-menu-submenu"]') === null,
      noOverlay: document.querySelector('.action-sheet.overlay') === null,
    };
  })()`);

  expect(result.parentExpanded).toBe('true');
  expect(result.childCount).toBeGreaterThan(0);
  expect(result.noFlyout).toBe(true);
  expect(result.noOverlay).toBe(true);
});

test('narrow: Move to space opens as in-place accordion inside the bottom-sheet', async ({ tauriPage }) => {
  const title = `e2e ctx-accordion-narrow ${Date.now()}`;

  await setWindowLogicalSize(tauriPage, 520, 720);
  await tauriPage.waitForFunction(`(() => window.innerWidth <= 640)()`, 5_000);

  await openContextMenuForQuest(tauriPage, title);

  // Narrow viewport → mobile sheet
  const isMobileSheet = await tauriPage.evaluate<boolean>(`(() =>
    document.querySelector('[data-testid="context-menu-sheet"]') !== null
  )()`);
  expect(isMobileSheet).toBe(true);

  // Click the "Move to space" parent row inside the sheet
  await tauriPage.evaluate(`(() => {
    const btn = Array.from(document.querySelectorAll('[data-testid="context-menu-sheet"] .sheet-item[aria-expanded]'))
      .find((el) => el.textContent?.includes('Move to space'));
    if (!(btn instanceof HTMLElement)) throw new Error('"Move to space" row not found in sheet');
    btn.click();
  })()`);

  await tauriPage.waitForFunction(`(() => {
    return document.querySelectorAll('[data-testid="context-menu-sheet"] .sheet-item.child').length > 0;
  })()`, 3_000);

  const result = await tauriPage.evaluate<{
    childCount: number;
    noOverlay: boolean;
    overlayBottomAboveComposer: boolean;
  }>(`(() => {
    const composer = document.querySelector('.chat-composer-bar');
    const composerTop = composer instanceof HTMLElement ? composer.getBoundingClientRect().top : window.innerHeight;
    const overlay = document.querySelector('.action-sheet.overlay');
    return {
      childCount: document.querySelectorAll('[data-testid="context-menu-sheet"] .sheet-item.child').length,
      noOverlay: overlay === null,
      overlayBottomAboveComposer: overlay
        ? overlay.getBoundingClientRect().bottom <= composerTop + 1
        : true,
    };
  })()`);

  expect(result.childCount).toBeGreaterThan(0);
  expect(result.noOverlay).toBe(true);
});

test('danger row (Delete) has a red wash on hover', async ({ tauriPage }) => {
  const title = `e2e ctx-danger ${Date.now()}`;
  await openContextMenuForQuest(tauriPage, title);

  const hasDangerAttr = await tauriPage.evaluate<boolean>(`(() => {
    const menu = document.querySelector('[data-testid="context-menu"]') ??
                 document.querySelector('[data-testid="context-menu-sheet"]');
    return !!menu?.querySelector('[data-danger]');
  })()`);
  expect(hasDangerAttr).toBe(true);
});
