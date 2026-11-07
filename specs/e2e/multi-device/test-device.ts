// The `testDevices` fixture (ADR-0013 D4): each test gets its test devices by
// name. A TestDevice is one running Fini app that the test drives. It is not
// the Fini `Device` model (a paired peer).

import { expect, test as base } from '@playwright/test';
import { TauriPage, type PluginClient } from '@srsholmes/tauri-playwright';
import { ChannelKind } from '../../../src/utils/channel.ts';
import { resetTestDevicesUi } from './helpers/teardown.ts';
import { TestDeviceSet, type TestDeviceProvider } from './test-device-set.ts';
import { TestDevicePlatform, TestDeviceSource } from './test-device-platform.ts';

export { TestDevicePlatform, TestDeviceSource };

const PAGE_TIMEOUT_MS = 15_000;
const READY_TIMEOUT_MS = 180_000;

export class TestDevice {
  readonly slug: string;
  readonly platform: TestDevicePlatform;
  readonly source: TestDeviceSource;
  private readonly provider: TestDeviceProvider;
  private client: PluginClient | null = null;
  private tauriPage: TauriPage | null = null;

  constructor(provider: TestDeviceProvider) {
    this.slug = provider.slug;
    this.platform = provider.platform;
    this.source = provider.source;
    this.provider = provider;
  }

  /** True for a test device that the harness started on `platform`. */
  isVirtual(platform: TestDevicePlatform): boolean {
    return this.source === TestDeviceSource.Virtual && this.platform === platform;
  }

  get page(): TauriPage {
    if (!this.tauriPage) throw new Error(`Test device "${this.slug}" is not started`);
    return this.tauriPage;
  }

  async start(): Promise<void> {
    await this.provider.prepare();
    await this.connect();
    // The ping proves only the Rust side of the bridge. A phone that starts
    // the app from zero shows the page some time later.
    await this.waitForPage();
  }

  private async connect(): Promise<void> {
    this.client = await this.provider.connect();
    this.tauriPage = new TauriPage(this.client);
    this.tauriPage.setDefaultTimeout(PAGE_TIMEOUT_MS);
  }

  private async waitForPage(): Promise<void> {
    const end = Date.now() + READY_TIMEOUT_MS;
    let lastError: unknown;
    while (Date.now() < end) {
      try {
        if (await this.page.evaluate<boolean>(`Boolean(document.querySelector('nav.nav'))`)) return;
      } catch (error) {
        lastError = error;
      }
      await new Promise((resolve) => setTimeout(resolve, 1_000));
    }
    const reason = lastError instanceof Error ? ` Last error: ${lastError.message}` : '';
    throw new Error(`Test device "${this.slug}" did not show the app in ${READY_TIMEOUT_MS / 1000} s.${reason}`);
  }

  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return this.page.evaluate<T>(`(async () => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (!invoke) throw new Error('Tauri invoke is unavailable');
      return await invoke(${JSON.stringify(command)}, ${JSON.stringify(args ?? {})});
    })()`);
  }

  /** Runs a shell command on the phone of a virtual Android test device. */
  shell(command: string): Promise<string> {
    if (!this.provider.shell) throw new Error(`Test device "${this.slug}" (${this.source} ${this.platform}) has no shell`);
    return this.provider.shell(command);
  }

  async stop(keepFiles: boolean): Promise<void> {
    try {
      this.client?.disconnect();
    } catch {
      // Best-effort disconnect only.
    }
    await this.provider.dispose(keepFiles);
  }
}

interface TestDeviceOptions {
  /**
   * The virtual test devices of a spec, slug → platform. Set it at the top of
   * a spec file with `test.use`. `FINI_E2E_TEST_DEVICES` gives other test
   * devices in place of it.
   */
  testDevicePlatforms: Record<string, TestDevicePlatform>;
  /**
   * The channel that the test devices use. Bluetooth on virtual Linux is the
   * mock radio. (Playwright has its own `channel` option, for the browser.)
   */
  testDeviceChannel: ChannelKind;
}

interface TestDeviceFixtures {
  testDevices: Record<string, TestDevice>;
  testDeviceA: TestDevice;
  testDeviceB: TestDevice;
}

/** The started set, or why this job cannot start it. */
type StartedSet = { set: TestDeviceSet; skipReason: null } | { set: null; skipReason: string };

interface TestDeviceWorkerFixtures {
  testDeviceSet: StartedSet;
}

export const test = base.extend<TestDeviceFixtures, TestDeviceWorkerFixtures & TestDeviceOptions>({
  testDevicePlatforms: [{ a: TestDevicePlatform.Linux, b: TestDevicePlatform.Linux }, { scope: 'worker', option: true }],
  testDeviceChannel: [ChannelKind.Network, { scope: 'worker', option: true }],

  testDeviceSet: [async ({ testDevicePlatforms, testDeviceChannel }, use) => {
    const set = TestDeviceSet.fromEnv(testDevicePlatforms, testDeviceChannel);
    // A worker fixture cannot skip a test, so the test fixture below does it.
    const skipReason = set.missingPlatformReason();
    if (skipReason) {
      await use({ set: null, skipReason });
      return;
    }
    await set.start((provider) => new TestDevice(provider));
    try {
      await use({ set, skipReason: null });
    } finally {
      await set.stop();
    }
  }, { scope: 'worker' }],

  testDevices: async ({ testDeviceSet }, use, testInfo) => {
    if (!testDeviceSet.set) {
      testInfo.skip(true, testDeviceSet.skipReason);
      return;
    }
    const testDevices = testDeviceSet.set.testDevices as Record<string, TestDevice>;
    try {
      await use(testDevices);
    } finally {
      await resetTestDevicesUi(Object.values(testDevices));
    }
  },

  testDeviceA: async ({ testDevices }, use) => {
    await use(byPosition(testDevices, 0));
  },

  testDeviceB: async ({ testDevices }, use) => {
    await use(byPosition(testDevices, 1));
  },
});

function byPosition(testDevices: Record<string, TestDevice>, index: number): TestDevice {
  const testDevice = Object.values(testDevices)[index];
  if (!testDevice) throw new Error(`no test device at position ${index}`);
  return testDevice;
}

export { expect };
