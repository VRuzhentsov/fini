// TestDeviceSet starts and stops the test devices of one worker. It reads
// which test devices to use, makes one provider for each, and owns the shared
// parts: the run folder and, in the `ble` lane, the mock broker.

import type { PluginClient } from '@srsholmes/tauri-playwright';
import { mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { BleMockBroker } from './providers/ble-mock-broker.ts';
import { DesktopProcessProvider, DesktopRun, type Transport } from './providers/desktop-process.ts';
import { AndroidPhoneProvider } from './providers/android-phone.ts';
import { PhysicalDeviceProvider } from './providers/physical-device.ts';
import { PhoneGroup } from './phones/phone-group.ts';
import { RUN_ROOT } from './run-root.ts';

export type TestDeviceKind = 'desktop-process' | 'android-phone' | 'physical-device';

/** How one test device is started, connected to and stopped. */
export interface TestDeviceProvider {
  readonly slug: string;
  readonly kind: TestDeviceKind;
  /** Brings the app to a state where `connect` can succeed. */
  prepare(): Promise<void>;
  connect(): Promise<PluginClient>;
  /** Releases only what this provider made. Keeps the logs when `keepFiles` is true. */
  dispose(keepFiles: boolean): Promise<void>;
}

interface TestDeviceEntry {
  slug: string;
  kind: TestDeviceKind;
  port?: number;
}

interface StartableTestDevice {
  readonly slug: string;
  start(): Promise<void>;
  stop(keepFiles: boolean): Promise<void>;
}

export class TestDeviceSet {
  readonly testDevices: Record<string, StartableTestDevice> = {};
  private readonly started: StartableTestDevice[] = [];
  private broker: BleMockBroker | null = null;
  private stopped = false;

  private constructor(
    private readonly entries: TestDeviceEntry[],
    private readonly phoneCount: number,
    private readonly runDir: string,
    private readonly transport: Transport,
  ) {}

  /** Reads `slug=kind,...` (see `testDeviceKinds` in test-device.ts). */
  static fromEnv(spec: string, phoneCount: number): TestDeviceSet {
    const entries = spec.split(',').map((item) => item.trim()).filter(Boolean).map(parseEntry);
    if (entries.length < 2) throw new Error(`a multi-device test needs at least two test devices, got "${spec}"`);
    const runId = process.env.FINI_E2E_RUN_ID ?? `${new Date().toISOString().replace(/[:.]/g, '-')}-${process.pid}`;
    const transport: Transport = process.env.FINI_E2E_TRANSPORT === 'ble' ? 'ble' : 'network';
    return new TestDeviceSet(entries, phoneCount, join(RUN_ROOT, runId), transport);
  }

  async start<T extends StartableTestDevice>(make: (provider: TestDeviceProvider) => T): Promise<void> {
    mkdirSync(this.runDir, { recursive: true });
    console.log(`FINI_E2E_RUN_DIR=${this.runDir}`);
    try {
      const providers = await this.providers();
      for (const provider of providers) {
        const testDevice = make(provider);
        await testDevice.start();
        this.started.push(testDevice);
        this.testDevices[testDevice.slug] = testDevice;
      }
    } catch (error) {
      await this.stop(true);
      throw error;
    }
  }

  async stop(keepFiles = false): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    const keep = keepFiles || process.env.FINI_E2E_KEEP === '1' || (process.exitCode ?? 0) !== 0;
    for (const testDevice of [...this.started].reverse()) {
      await testDevice.stop(keep).catch(() => undefined);
    }
    // After the test devices: nothing needs the mock radio any more.
    await this.broker?.stop();
    if (keep) {
      console.log(`Keeping the e2e run folder: ${this.runDir}`);
    } else {
      rmSync(this.runDir, { recursive: true, force: true });
    }
  }

  private async providers(): Promise<TestDeviceProvider[]> {
    const desktop = this.entries.filter((entry) => entry.kind === 'desktop-process');
    const phones = this.entries.filter((entry) => entry.kind === 'android-phone');

    let desktopRun: DesktopRun | null = null;
    if (desktop.length > 0) {
      desktopRun = new DesktopRun(this.runDir, this.entries.map((entry) => entry.slug), this.transport);
      if (this.transport === 'ble') {
        this.broker = new BleMockBroker(this.runDir, desktopRun.brokerPort);
        await this.broker.start();
      }
    }

    // Only here does a phone start, and only when a test device is a phone
    // (ADR-0012 D7).
    const phoneSerials = phones.length > 0
      ? await PhoneGroup.shared().serials(Math.max(this.phoneCount, phones.length))
      : [];

    return this.entries.map((entry, index) => {
      switch (entry.kind) {
        case 'desktop-process':
          return new DesktopProcessProvider(entry.slug, index, desktopRun!);
        case 'android-phone': {
          const phoneIndex = phones.indexOf(entry);
          return new AndroidPhoneProvider(entry.slug, phoneSerials[phoneIndex], phoneIndex, this.runDir);
        }
        case 'physical-device':
          return new PhysicalDeviceProvider(entry.slug, entry.port!);
      }
    });
  }
}

function parseEntry(item: string): TestDeviceEntry {
  const [slug, kind] = item.split('=').map((part) => part.trim());
  if (!slug || !kind) throw new Error(`a test device must be "slug=kind", got "${item}"`);
  if (kind === 'desktop-process' || kind === 'android-phone') return { slug, kind };
  const tcp = /^tcp:(\d+)$/.exec(kind);
  if (tcp) return { slug, kind: 'physical-device', port: Number(tcp[1]) };
  throw new Error(`unknown test device kind "${kind}" in "${item}" (use desktop-process, android-phone or tcp:<port>)`);
}
