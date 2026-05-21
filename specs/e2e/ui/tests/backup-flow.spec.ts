import { test, expect } from '../fixtures.ts';

interface Space { id: string; name: string }
interface PreflightResult { required_space_mappings: unknown[]; conflicts: unknown[] }

async function invokeTauri<T>(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return tauriPage.evaluate<T>(`(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error('Tauri invoke is unavailable');
    return await invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args ?? {})});
  })()`);
}

/**
 * Set window globals read by the frontend's E2E hooks to bypass native OS dialogs.
 * open → returned by useBackupImport's startImport()
 * save → returned by ExportSpacesDialog's exportSelected()
 */
async function setDialogPath(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  patches: { open?: string | null; save?: string | null },
): Promise<void> {
  await tauriPage.evaluate(`(() => {
    ${patches.open !== undefined ? `window.__FINI_E2E_OPEN_PATH__ = ${JSON.stringify(patches.open)};` : ''}
    ${patches.save !== undefined ? `window.__FINI_E2E_SAVE_PATH__ = ${JSON.stringify(patches.save)};` : ''}
  })()`);
}

async function clickRowButton(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  testId: string,
): Promise<void> {
  await tauriPage.evaluate(`(() => {
    const row = document.querySelector('[data-testid="${testId}"]');
    if (!(row instanceof HTMLElement)) throw new Error('${testId} not found');
    row.scrollIntoView({ behavior: 'instant', block: 'center' });
    const btn = row.querySelector('button') ?? row;
    (btn instanceof HTMLElement ? btn : row).click();
  })()`);
}

async function navigateToSettings(
  tauriPage: { waitForSelector: (s: string, t?: number) => Promise<unknown>; evaluate: <R>(s: string) => Promise<R> },
): Promise<void> {
  await tauriPage.waitForSelector('nav.nav', 30_000);
  await tauriPage.evaluate(`(() => { window.location.hash = '#/settings'; })()`);
  await tauriPage.waitForSelector('[data-testid="settings-backup"]', 10_000);
}

test('Settings page renders backup section with export and import rows', async ({ tauriPage }) => {
  await navigateToSettings(tauriPage);

  expect(await tauriPage.isVisible('[data-testid="backup-export-row"]')).toBe(true);
  expect(await tauriPage.isVisible('[data-testid="backup-import-row"]')).toBe(true);
});

test('export: dialog opens, select-all enables Export button, export completes with toast', async ({ tauriPage }) => {
  const exportPath = '/var/tmp/fini-e2e-export-ui.zip';

  await navigateToSettings(tauriPage);
  await setDialogPath(tauriPage, { save: exportPath });

  // Click the inner button inside the export row
  await clickRowButton(tauriPage, 'backup-export-row');
  await tauriPage.waitForSelector('[data-testid="export-spaces-dialog"]', 5_000);

  // Export button is disabled until a space is selected
  const exportBtnDisabled = await tauriPage.evaluate<boolean>(`(() => {
    const btns = Array.from(document.querySelectorAll('[data-testid="export-spaces-dialog"] button'));
    const btn = btns.find(b => b.textContent?.trim().startsWith('Export'));
    return btn instanceof HTMLButtonElement && btn.disabled;
  })()`);
  expect(exportBtnDisabled).toBe(true);

  // Click "Select all"
  await tauriPage.evaluate(`(() => {
    const btns = Array.from(document.querySelectorAll('[data-testid="export-spaces-dialog"] button'));
    const btn = btns.find(b => b.textContent?.trim() === 'Select all');
    if (!(btn instanceof HTMLElement)) throw new Error('Select all button not found');
    btn.click();
  })()`);

  // Export button is now enabled with space count
  await tauriPage.waitForFunction(`(() => {
    const btns = Array.from(document.querySelectorAll('[data-testid="export-spaces-dialog"] button'));
    const btn = btns.find(b => b.textContent?.trim().startsWith('Export'));
    return btn instanceof HTMLButtonElement && !btn.disabled;
  })()`, 5_000);

  // Click Export — save path is intercepted via window global, backup_export runs, toast appears
  await tauriPage.evaluate(`(() => {
    const btns = Array.from(document.querySelectorAll('[data-testid="export-spaces-dialog"] button'));
    const btn = btns.find(b => b.textContent?.trim().startsWith('Export'));
    if (!(btn instanceof HTMLElement)) throw new Error('Export button not found');
    btn.click();
  })()`);

  await tauriPage.waitForFunction(`(() =>
    document.body.textContent?.includes('Backup exported')
  )()`, 8_000);

  const hasOpenLocation = await tauriPage.evaluate<boolean>(`(() =>
    document.body.textContent?.includes('Open location') ?? false
  )()`);
  expect(hasOpenLocation).toBe(true);
});

test('import: picking a backup with no conflicts auto-applies and shows toast', async ({ tauriPage }) => {
  const backupPath = '/var/tmp/fini-e2e-import-ui.zip';
  const spaces = await invokeTauri<Space[]>(tauriPage, 'get_spaces');
  await invokeTauri<void>(tauriPage, 'backup_export', { path: backupPath, spaceIds: spaces.map(s => s.id) });

  // Verify preflight has no conflicts (the import should auto-apply)
  const preflight = await invokeTauri<PreflightResult>(tauriPage, 'backup_preflight_import', {
    path: backupPath,
    mappings: [],
  });
  expect(preflight.required_space_mappings).toHaveLength(0);
  expect(preflight.conflicts).toHaveLength(0);

  await navigateToSettings(tauriPage);
  await setDialogPath(tauriPage, { open: backupPath });

  // Click the inner button inside the import row — open() intercepted, preflight runs, auto-apply fires
  await clickRowButton(tauriPage, 'backup-import-row');

  await tauriPage.waitForFunction(`(() =>
    document.body.textContent?.includes('Backup imported')
  )()`, 10_000);
});
