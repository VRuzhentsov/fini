// TestDeviceSet starts and stops the test devices of one worker. It reads
// which test devices to use, makes one provider for each, and owns the shared
// parts: the run folder and, for Bluetooth on virtual Linux test devices, the
// mock broker.

import type { PluginClient } from '@srsholmes/tauri-playwright';
import { mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { ChannelKind } from '../../../src/utils/channel.ts';
import { E2eEnv } from '../e2e-project.ts';
import { BleMockBroker } from './providers/ble-mock-broker.ts';
import { LinuxProvider, LinuxRun } from './providers/linux.ts';
import { AndroidProvider } from './providers/android.ts';
import { TcpProvider } from './providers/tcp.ts';
import { PhoneGroup } from './phones/phone-group.ts';
import { RUN_ROOT } from './run-root.ts';
import { TestDevicePlatform, TestDeviceSource } from './test-device-platform.ts';

/** How one test device is started, connected to and stopped. */
export interface TestDeviceProvider {
  readonly slug: string;
  readonly platform: TestDevicePlatform;
  readonly source: TestDeviceSource;
  /** Brings the app to a state where `connect` can succeed. */
  prepare(): Promise<void>;
  connect(): Promise<PluginClient>;
  /** Releases only what this provider made. Keeps the logs when `keepFiles` is true. */
  dispose(keepFiles: boolean): Promise<void>;
  /** Runs a shell command on the phone and gives its output. Only a virtual Android test device has it. */
  shell?(command: string): Promise<string>;
}

interface TestDeviceEntry {
  slug: string;
  platform: TestDevicePlatform;
  source: TestDeviceSource;
  port?: number;
}

interface StartableTestDevice {
  readonly slug: string;
  start(): Promise<void>;
  stop(keepFiles: boolean): Promise<void>;
}

const DEFAULT_VIRTUAL_PLATFORMS: TestDevicePlatform[] = [TestDevicePlatform.Linux];
const PLATFORMS = Object.values(TestDevicePlatform);

export class TestDeviceSet {
  readonly testDevices: Record<string, StartableTestDevice> = {};
  private readonly started: StartableTestDevice[] = [];
  private broker: BleMockBroker | null = null;
  private stopped = false;

  private constructor(
    private readonly entries: TestDeviceEntry[],
    private readonly channel: ChannelKind,
    private readonly runDir: string,
  ) {}

  /**
   * The test devices of `FINI_E2E_TEST_DEVICES` when it is set, else the
   * virtual test devices of `platforms` (slug → platform).
   */
  static fromEnv(platforms: Record<string, TestDevicePlatform>, channel: ChannelKind): TestDeviceSet {
    const fromEnv = process.env[E2eEnv.TestDevices];
    const entries = fromEnv
      ? fromEnv.split(',').map((item) => item.trim()).filter(Boolean).map(parseEntry)
      : Object.entries(platforms).map(([slug, platform]) => ({ slug, platform, source: TestDeviceSource.Virtual }));
    if (entries.length < 2) throw new Error(`a multi-device test needs at least two test devices, got ${entries.length}`);
    const runId = process.env.FINI_E2E_RUN_ID ?? `${new Date().toISOString().replace(/[:.]/g, '-')}-${process.pid}`;
    return new TestDeviceSet(entries, channel, join(RUN_ROOT, runId));
  }

  /**
   * Why this job cannot start the set, or null when it can. A job starts only
   * the virtual platforms in `FINI_E2E_VIRTUAL_PLATFORMS`; a test device with
   * the physical source runs already.
   */
  missingPlatformReason(): string | null {
    const listed = process.env[E2eEnv.VirtualPlatforms];
    const available = listed
      ? listed.split(',').map((item) => item.trim()).filter(Boolean)
      : DEFAULT_VIRTUAL_PLATFORMS;
    const missing = [...new Set(this.entries
      .filter((entry) => entry.source === TestDeviceSource.Virtual && !available.includes(entry.platform))
      .map((entry) => entry.platform))];
    if (missing.length === 0) return null;
    return `needs a virtual ${missing.join(', ')}; this job starts ${available.join(', ')} (${E2eEnv.VirtualPlatforms})`;
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
    const keep = keepFiles || process.env[E2eEnv.Keep] === '1' || (process.exitCode ?? 0) !== 0;
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
    const virtual = (platform: TestDevicePlatform) => this.entries.filter(
      (entry) => entry.source === TestDeviceSource.Virtual && entry.platform === platform,
    );
    const linux = virtual(TestDevicePlatform.Linux);
    const phones = virtual(TestDevicePlatform.Android);

    let linuxRun: LinuxRun | null = null;
    if (linux.length > 0) {
      linuxRun = new LinuxRun(this.runDir, this.entries.map((entry) => entry.slug), this.channel);
      if (this.channel === ChannelKind.Bluetooth) {
        this.broker = new BleMockBroker(this.runDir, linuxRun.brokerPort);
        await this.broker.start();
      }
    }

    // Only here does a phone start, and only when a test device is a virtual
    // Android (ADR-0013 D7).
    const phoneSerials = phones.length > 0 ? await PhoneGroup.shared().serials(phones.length) : [];

    return this.entries.map((entry, index) => {
      if (entry.source === TestDeviceSource.Physical) return new TcpProvider(entry.slug, entry.platform, entry.port!);
      switch (entry.platform) {
        case TestDevicePlatform.Linux:
          return new LinuxProvider(entry.slug, index, linuxRun!);
        case TestDevicePlatform.Android: {
          const phoneIndex = phones.indexOf(entry);
          return new AndroidProvider(entry.slug, phoneSerials[phoneIndex], phoneIndex, this.runDir);
        }
      }
    });
  }
}

/** Reads `slug=<platform>` (virtual) or `slug=<platform>@tcp:<port>` (physical). */
function parseEntry(item: string): TestDeviceEntry {
  const [slug, value] = item.split('=').map((part) => part.trim());
  if (!slug || !value) throw new Error(`a test device must be "slug=<platform>[@tcp:<port>]", got "${item}"`);
  const [platform, address] = value.split('@');
  if (!isPlatform(platform)) {
    throw new Error(`unknown platform "${platform}" in "${item}" (use ${PLATFORMS.join(', ')})`);
  }
  if (address === undefined) return { slug, platform, source: TestDeviceSource.Virtual };
  const tcp = /^tcp:(\d+)$/.exec(address);
  if (!tcp) throw new Error(`a physical test device must be "${slug}=${platform}@tcp:<port>", got "${item}"`);
  return { slug, platform, source: TestDeviceSource.Physical, port: Number(tcp[1]) };
}

function isPlatform(value: string): value is TestDevicePlatform {
  return (PLATFORMS as string[]).includes(value);
}
