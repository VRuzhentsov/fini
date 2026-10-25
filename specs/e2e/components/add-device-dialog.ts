import type { TauriPage } from '@srsholmes/tauri-playwright';
import type { ChannelKind } from '../../../src/utils/channel.ts';
import { waitForFunction, waitForSelector } from './wait.ts';

const DIALOG = '[data-testid="pair-device-dialog"]';
const DEFAULT_TIMEOUT_MS = 30_000;

/** A step of the dialog, as `data-pair-step` shows it (`DeviceSetupDialog.vue`). */
export type PairStep = 'channel' | 'looking' | 'sent' | 'code' | 'incoming' | 'entercode' | 'paired';

/**
 * The Add device dialog on Settings, used as a person uses it: each method is
 * one action on the screen, and it returns when the screen shows the result.
 */
export class AddDeviceDialog {
  constructor(
    private readonly page: TauriPage,
    private readonly timeoutMs = DEFAULT_TIMEOUT_MS,
  ) {}

  /** Opens Add device from Settings and starts the search on `channel`. */
  async openSearch(channel: ChannelKind): Promise<void> {
    await waitForSelector(this.page,'nav.nav a[href="#/settings"]', this.timeoutMs);
    await this.page.click('nav.nav a[href="#/settings"]');
    await waitForSelector(this.page,'[data-testid="add-device-link"]', this.timeoutMs);
    await this.page.click('[data-testid="add-device-link"]');
    await this.waitForStep('channel');
    await this.page.click(`[data-testid="pair-channel-${channel}"]`);
    await this.waitForStep('looking');
  }

  /** Waits until the list shows the device called `hostname`. */
  async waitForDevice(hostname: string, timeoutMs = this.timeoutMs): Promise<void> {
    await waitForSelector(this.page,this.deviceRow(hostname), timeoutMs);
  }

  /** Presses Pair on `hostname`'s row. */
  async pair(hostname: string): Promise<void> {
    await this.page.click(`${this.deviceRow(hostname)} [data-testid="request-pair"]`);
  }

  /** Waits until a pair request from another device is on the screen. */
  async waitForRequest(timeoutMs = this.timeoutMs): Promise<void> {
    await this.waitForStep('incoming', timeoutMs);
  }

  /** Accepts the request on the screen; the code field comes next. */
  async accept(): Promise<void> {
    await this.page.click(`${DIALOG} [data-testid="accept-incoming-request"]`);
    await this.waitForStep('entercode');
  }

  /** The code this device shows, once the other device accepted. */
  async code(timeoutMs = this.timeoutMs): Promise<string> {
    await this.waitForStep('code', timeoutMs);
    return (await this.page.allTextContents('[data-testid="pair-code"]')).map((digit) => digit.trim()).join('');
  }

  /** Types `code` and presses Pair. */
  async typeCode(code: string): Promise<void> {
    await this.page.fill('[data-testid="pair-code-input"]', code);
    await this.page.click('[data-testid="pair-code-submit"]');
  }

  /** Waits until the dialog shows Paired. */
  async waitForPaired(timeoutMs = this.timeoutMs): Promise<void> {
    await this.waitForStep('paired', timeoutMs);
  }

  /** Presses Done; returns when the dialog is closed. */
  async done(): Promise<void> {
    await this.page.click('[data-testid="pair-done"]');
    await this.waitForClosed();
  }

  /** Closes the dialog from any step, by its backdrop. */
  async close(): Promise<void> {
    if ((await this.page.count(DIALOG)) === 0) return;
    await this.page.click('[data-testid="pair-dialog-backdrop"]');
    await this.waitForClosed();
  }

  private async waitForStep(step: PairStep, timeoutMs = this.timeoutMs): Promise<void> {
    await waitForSelector(this.page,`${DIALOG}[data-pair-step="${step}"]`, timeoutMs);
  }

  private async waitForClosed(): Promise<void> {
    await waitForFunction(this.page, `!document.querySelector(${JSON.stringify(DIALOG)})`, this.timeoutMs);
  }

  private deviceRow(hostname: string): string {
    return `${DIALOG} [data-testid="nearby-device-row"][data-device-hostname=${JSON.stringify(hostname)}]`;
  }
}
