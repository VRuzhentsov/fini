import type { TauriPage } from '@srsholmes/tauri-playwright';
import { ChannelColor, type ChannelKind } from '../../../src/utils/channel.ts';
import { ChannelSetupDialog } from './channel-setup-dialog.ts';
import { waitForSelector } from './wait.ts';

const DEFAULT_TIMEOUT_MS = 30_000;

/** The page of one paired device (Settings -> the device's row). */
export class DevicePage {
  constructor(
    private readonly page: TauriPage,
    private readonly timeoutMs = DEFAULT_TIMEOUT_MS,
  ) {}

  /** Opens the page of `peerDeviceId` from Settings, as a person does. */
  async open(peerDeviceId: string): Promise<void> {
    await waitForSelector(this.page,'nav.nav a[href="#/settings"]', this.timeoutMs);
    await this.page.click('nav.nav a[href="#/settings"]');
    await waitForSelector(this.page,'[data-testid="settings-devices"]', this.timeoutMs);
    await this.page.click(`[data-testid="paired-device-row"][data-peer-device-id=${JSON.stringify(peerDeviceId)}] a`);
    await waitForSelector(this.page,'[data-testid="channel-status-row"]', this.timeoutMs);
  }

  /** Unlink, then Unlink in the dialog: the pair is removed on this device. */
  async unlinkDevice(): Promise<void> {
    await this.page.click('[data-testid="unlink-device"]');
    await waitForSelector(this.page,'[data-testid="unlink-device-confirm"]', this.timeoutMs);
    await this.page.click('[data-testid="unlink-device-confirm"]');
    await waitForSelector(this.page,'[data-testid="settings-devices"]', this.timeoutMs);
  }

  /** The row of `kind` on this page. */
  channel(kind: ChannelKind): ChannelRow {
    return new ChannelRow(this.page, kind, this.timeoutMs);
  }
}

/** One channel row on a device page: its colour, its switch, Unlink and Add. */
export class ChannelRow {
  constructor(
    private readonly page: TauriPage,
    private readonly kind: ChannelKind,
    private readonly timeoutMs = DEFAULT_TIMEOUT_MS,
  ) {}

  private get row(): string {
    return `[data-testid="channel-status-row"][data-channel-kind="${this.kind}"]`;
  }

  /** The colour the row shows now. */
  async color(): Promise<ChannelColor | null> {
    return (await this.page.getAttribute(this.row, 'data-channel-color')) as ChannelColor | null;
  }

  /** Waits until the row shows `color`. */
  async waitForColor(color: ChannelColor, timeoutMs = this.timeoutMs): Promise<void> {
    await waitForSelector(this.page,`${this.row}[data-channel-color="${color}"]`, timeoutMs);
  }

  /** Switches the channel off, if it is on; returns when Unlink is offered. */
  async switchOff(): Promise<void> {
    if ((await this.color()) !== ChannelColor.Off) {
      await this.page.click(`${this.row} [data-testid="channel-switch"]`);
    }
    await waitForSelector(this.page,`${this.row} [data-testid="unlink-channel"]`, this.timeoutMs);
  }

  /**
   * Presses Unlink; returns when the row offers Add. The page drops a press
   * while the row's switch is still settling, so the press is made again
   * until Add shows.
   */
  async unlink(): Promise<void> {
    const add = `${this.row} [data-testid="add-channel"]`;
    const unlink = `${this.row} [data-testid="unlink-channel"]`;
    const deadline = Date.now() + this.timeoutMs;
    while ((await this.page.count(add)) === 0) {
      if (Date.now() > deadline) throw new Error(`${this.kind} row never offered Add after Unlink`);
      if ((await this.page.count(unlink)) > 0 && (await this.page.isEnabled(unlink))) {
        await this.page.click(unlink);
      }
      await waitForSelector(this.page,add, 1_000).catch(() => undefined);
    }
  }

  /** Presses Add; the channel's setup dialog opens. */
  async add(): Promise<ChannelSetupDialog> {
    await this.page.click(`${this.row} [data-testid="add-channel"]`);
    await waitForSelector(this.page,'[data-testid="pair-device-dialog"]', this.timeoutMs);
    return new ChannelSetupDialog(this.page, this.timeoutMs);
  }

  /** Waits until the row offers Add: the channel is not set up on this device. */
  async waitForAdd(timeoutMs = this.timeoutMs): Promise<void> {
    await waitForSelector(this.page,`${this.row} [data-testid="add-channel"]`, timeoutMs);
  }
}
