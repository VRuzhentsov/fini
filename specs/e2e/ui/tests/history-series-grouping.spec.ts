import { test, expect } from '../fixtures.ts';

interface Quest {
  id: string;
  title: string;
  status: 'active' | 'completed' | 'abandoned';
  series_id: string | null;
}

async function invokeTauri<T>(tauriPage: { evaluate: <R>(script: string) => Promise<R> }, cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriPage.evaluate<T>(`(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error('Tauri invoke is unavailable');
    return await invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args ?? {})});
  })()`);
}

async function invokeTauriRetry<T>(tauriPage: { evaluate: <R>(script: string) => Promise<R> }, cmd: string, args?: Record<string, unknown>): Promise<T> {
  let lastError: unknown;
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      return await invokeTauri<T>(tauriPage, cmd, args);
    } catch (error) {
      lastError = error;
      if (!String(error).includes('database is locked')) throw error;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  throw lastError;
}

test('History groups same-series resolved occurrences and deletes the series', async ({ tauriPage }) => {
  const title = `e2e history repeat ${Date.now()}`;
  const repeatRule = JSON.stringify({ preset: 'daily' });

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  const first = await invokeTauriRetry<Quest>(tauriPage, 'create_quest', {
    input: { title, repeat_rule: repeatRule, due: '2026-05-01' },
  });
  expect(first.series_id).toBeTruthy();

  await invokeTauriRetry<Quest>(tauriPage, 'update_quest', { id: first.id, input: { status: 'completed' } });
  let quests = await invokeTauriRetry<Quest[]>(tauriPage, 'get_quests');
  const second = quests.find((quest) => quest.title === title && quest.status === 'active');
  expect(second).toBeTruthy();

  await invokeTauriRetry<Quest>(tauriPage, 'update_quest', { id: second!.id, input: { status: 'abandoned' } });

  await tauriPage.click('nav.nav a[href="#/history"]');
  await tauriPage.waitForSelector('[data-testid="quest-row-group-header"]', 10_000);

  const groupedRowText = await tauriPage.evaluate<string>(`(() => document.querySelector('[data-testid="quest-row-group-header"]')?.textContent ?? '')()`);
  expect(groupedRowText).toMatch(/Completed|Abandoned|Mixed/);
  expect(groupedRowText).not.toContain('2x');

  // Expand children via the group expander (chevron)
  await tauriPage.click('[data-testid="quest-row-group-expander"]');
  await tauriPage.waitForSelector('[data-testid="quest-row-group-children"]', 5_000);
  const childCount = await tauriPage.evaluate<number>(`(() => document.querySelectorAll('[data-testid="quest-row-group-children"] li').length)()`);
  expect(childCount).toBe(2);

  // Confirm-cancel path: series must survive
  await tauriPage.installDialogHandler({ defaultConfirm: false });
  await tauriPage.evaluate(`(() => {
    const header = document.querySelector('[data-testid="quest-row-group-header"]');
    if (!(header instanceof HTMLElement)) throw new Error('Group header not found');
    const rect = header.getBoundingClientRect();
    header.dispatchEvent(new MouseEvent('contextmenu', {
      bubbles: true, cancelable: true,
      clientX: rect.left + rect.width / 2,
      clientY: rect.top + rect.height / 2,
    }));
  })()`);
  await tauriPage.waitForSelector('[data-testid="context-menu"], [data-testid="context-menu-sheet"]', 5_000);
  await tauriPage.evaluate(`(() => {
    const menu = document.querySelector('[data-testid="context-menu"]') ?? document.querySelector('[data-testid="context-menu-sheet"]');
    const row = Array.from(menu?.querySelectorAll('.sheet-item') ?? []).find((candidate) => candidate.textContent?.includes('Delete series'));
    if (!(row instanceof HTMLElement)) throw new Error('Delete series menu item not found');
    row.click();
  })()`);
  const cancelDialogs = await tauriPage.getDialogs();
  expect(cancelDialogs.some((d: { type: string }) => d.type === 'confirm')).toBe(true);
  await tauriPage.clearDialogs();
  // Row must still be visible (cancel path)
  await tauriPage.waitForSelector('[data-testid="quest-row-group-header"]', 3_000);

  // Confirm-ok path: series must be removed.
  // Use tauriPage.click (plugin native click, not JS element.click()) so the GLib
  // main-loop dialog signal is processed synchronously within the click command.
  // installDialogHandler must be called before the click so the handler is ready.
  await tauriPage.installDialogHandler({ defaultConfirm: true });
  await tauriPage.evaluate(`(() => {
    const header = document.querySelector('[data-testid="quest-row-group-header"]');
    if (!(header instanceof HTMLElement)) throw new Error('Group header not found');
    const rect = header.getBoundingClientRect();
    header.dispatchEvent(new MouseEvent('contextmenu', {
      bubbles: true, cancelable: true,
      clientX: rect.left + rect.width / 2,
      clientY: rect.top + rect.height / 2,
    }));
  })()`);
  // The "Delete series" item is the only .sheet-item[data-danger] in the menu.
  await tauriPage.waitForSelector('.sheet-item[data-danger]', 5_000);
  await tauriPage.click('.sheet-item[data-danger]');

  // Poll backend until the quests are gone; the delete can take several seconds
  // under SQLite write-lock contention in CI.
  const deadline = Date.now() + 30_000;
  while (true) {
    quests = await invokeTauriRetry<Quest[]>(tauriPage, 'get_quests');
    if (!quests.some((q: Quest) => q.series_id === first.series_id)) break;
    if (Date.now() >= deadline) throw new Error('Series was not deleted within 30 s');
    await new Promise<void>((r) => setTimeout(r, 1_000));
  }
  expect(quests.some((quest) => quest.series_id === first.series_id)).toBe(false);
  // Verify the DOM also updated reactively
  await tauriPage.waitForFunction(`(() => !document.querySelector('[data-testid="quest-row-group-header"]'))()`, 10_000);
});

test('History header shows Mixed status when children disagree', async ({ tauriPage }) => {
  const title = `e2e mixed status ${Date.now()}`;
  const repeatRule = JSON.stringify({ preset: 'daily' });

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  const first = await invokeTauriRetry<Quest>(tauriPage, 'create_quest', {
    input: { title, repeat_rule: repeatRule, due: '2026-05-01' },
  });
  expect(first.series_id).toBeTruthy();

  // Complete first occurrence → auto-generates a next active one
  await invokeTauriRetry<Quest>(tauriPage, 'update_quest', { id: first.id, input: { status: 'completed' } });
  const quests = await invokeTauriRetry<Quest[]>(tauriPage, 'get_quests');
  const second = quests.find((q) => q.title === title && q.status === 'active');
  expect(second).toBeTruthy();

  // Abandon the second occurrence → two resolved rows with different statuses
  await invokeTauriRetry<Quest>(tauriPage, 'update_quest', { id: second!.id, input: { status: 'abandoned' } });

  await tauriPage.click('nav.nav a[href="#/history"]');
  await tauriPage.waitForSelector('[data-testid="quest-row-group-header"]', 10_000);

  const headerText = await tauriPage.evaluate<string>(`(() => document.querySelector('[data-testid="quest-row-group-header"]')?.textContent ?? '')()`);
  expect(headerText).toContain('Mixed');
});
