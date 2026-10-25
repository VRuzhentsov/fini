import type { TauriPage } from '@srsholmes/tauri-playwright';
import { waitForFunction, waitForSelector } from './wait.ts';

const DIALOG = '[data-testid="pair-device-dialog"]';
const DEFAULT_TIMEOUT_MS = 30_000;

/**
 * The dialog that Add on a channel row opens: the two devices of a pair
 * search for each other, and OK becomes available when both have found the
 * other (`data-pair-step="setupReady"`).
 */
export class ChannelSetupDialog {
  constructor(
    private readonly page: TauriPage,
    private readonly timeoutMs = DEFAULT_TIMEOUT_MS,
  ) {}

  /** Waits until OK is available, or throws with what the dialog shows. */
  async waitForOk(timeoutMs = this.timeoutMs): Promise<void> {
    await waitForSelector(
      this.page,
      `${DIALOG}[data-pair-step="setupReady"], ${DIALOG}[data-pair-step="setupFailed"]`,
      timeoutMs,
    );
    const failure = await this.failure();
    if (failure) throw new Error(`channel setup failed: ${failure}`);
  }

  /** The failure the dialog shows, or '' when it shows none. */
  async failure(): Promise<string> {
    if ((await this.page.count('[data-testid="setup-failed"]')) === 0) return '';
    return ((await this.page.textContent('[data-testid="setup-failed"]')) ?? '').trim();
  }

  /** Presses OK: the channel is switched on. Returns when the dialog is closed. */
  async ok(): Promise<void> {
    await this.page.click('[data-testid="setup-ok"]');
    await waitForFunction(this.page, `!document.querySelector(${JSON.stringify(DIALOG)})`, this.timeoutMs);
  }

  /** Presses Close: the channel stays as it was. */
  async close(): Promise<void> {
    await this.page.click('[data-testid="setup-close"]');
  }
}
