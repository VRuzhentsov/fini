import type { TauriPage } from '@srsholmes/tauri-playwright';

/**
 * The longest wait that one `waitForSelector` / `waitForFunction` call of the
 * plugin keeps: a longer timeout ends at 30 s ("timeout (30s)").
 */
const PLUGIN_WAIT_LIMIT_MS = 25_000;

/** Waits up to `timeoutMs` for `selector`, also past the plugin's limit. */
export async function waitForSelector(page: TauriPage, selector: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const left = deadline - Date.now();
    try {
      await page.waitForSelector(selector, Math.max(1, Math.min(left, PLUGIN_WAIT_LIMIT_MS)));
      return;
    } catch (error) {
      if (Date.now() >= deadline) throw new Error(`timed out after ${timeoutMs} ms waiting for ${selector}`, { cause: error });
    }
  }
}

/** Waits up to `timeoutMs` until `expression` is true, also past the plugin's limit. */
export async function waitForFunction(page: TauriPage, expression: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const left = deadline - Date.now();
    try {
      await page.waitForFunction(expression, Math.max(1, Math.min(left, PLUGIN_WAIT_LIMIT_MS)));
      return;
    } catch (error) {
      if (Date.now() >= deadline) throw new Error(`timed out after ${timeoutMs} ms waiting for ${expression}`, { cause: error });
    }
  }
}
