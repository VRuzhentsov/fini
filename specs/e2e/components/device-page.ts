import type { TauriPage } from '@srsholmes/tauri-playwright';
import { ChannelColor, type ChannelKind } from '../../../src/utils/channel.ts';
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

/** One channel row on a device page: its colour, and its switch. */
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

  /** Switches the channel off, if it is on; returns when the row is off. */
  async switchOff(): Promise<void> {
    if ((await this.color()) !== ChannelColor.Off) {
      await this.page.click(`${this.row} [data-testid="channel-switch"]`);
    }
    await this.waitForColor(ChannelColor.Off);
  }

  /** Switches the channel on, if it is off (ADR-0011 D1). */
  async switchOn(): Promise<void> {
    if ((await this.color()) === ChannelColor.Off) {
      await this.page.click(`${this.row} [data-testid="channel-switch"]`);
    }
  }
}
