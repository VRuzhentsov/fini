// The `testDevices` fixture (ADR-0012 D4): each test gets its test devices by
// name. A TestDevice is one running Fini app that the test drives. It is not
// the Fini `Device` model (a paired peer).

import { expect, test as base } from '@playwright/test';
import { TauriPage, type PluginClient } from '@srsholmes/tauri-playwright';
import { resetTestDevicesUi } from './helpers/teardown.ts';
import { TestDeviceSet, type TestDeviceKind, type TestDeviceProvider } from './test-device-set.ts';

export type { TestDeviceKind };

const PAGE_TIMEOUT_MS = 15_000;
const READY_TIMEOUT_MS = 180_000;

export class TestDevice {
  readonly slug: string;
  readonly kind: TestDeviceKind;
  private readonly provider: TestDeviceProvider;
  private client: PluginClient | null = null;
  private tauriPage: TauriPage | null = null;

  constructor(provider: TestDeviceProvider) {
    this.slug = provider.slug;
    this.kind = provider.kind;
    this.provider = provider;
  }

  get page(): TauriPage {
    if (!this.tauriPage) throw new Error(`Test device "${this.slug}" is not started`);
    return this.tauriPage;
  }

  async start(): Promise<void> {
    await this.provider.prepare();
    this.client = await this.provider.connect();
    this.tauriPage = new TauriPage(this.client);
    this.tauriPage.setDefaultTimeout(PAGE_TIMEOUT_MS);
    // The ping proves only the Rust side of the bridge. A phone that starts
    // the app from zero shows the page some time later.
    await this.waitForPage();
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
   * The test devices of a project when `FINI_E2E_TEST_DEVICES` is not set, as
   * `slug=kind,...`. A kind is `desktop-process`, `android-phone` or
   * `tcp:<port>` (a physical device).
   */
  testDeviceKinds: string;
  /** How many virtual phones start in one group, for projects with `android-phone` test devices. */
  phoneCount: number;
}

interface TestDeviceFixtures {
  testDevices: Record<string, TestDevice>;
  testDeviceA: TestDevice;
  testDeviceB: TestDevice;
}

interface TestDeviceWorkerFixtures {
  testDeviceSet: TestDeviceSet;
}

export const test = base.extend<TestDeviceFixtures, TestDeviceWorkerFixtures & TestDeviceOptions>({
  testDeviceKinds: ['a=desktop-process,b=desktop-process', { scope: 'worker', option: true }],
  phoneCount: [2, { scope: 'worker', option: true }],

  testDeviceSet: [async ({ testDeviceKinds, phoneCount }, use) => {
    const set = TestDeviceSet.fromEnv(process.env.FINI_E2E_TEST_DEVICES ?? testDeviceKinds, phoneCount);
    await set.start((provider) => new TestDevice(provider));
    try {
      await use(set);
    } finally {
      await set.stop();
    }
  }, { scope: 'worker' }],

  testDevices: async ({ testDeviceSet }, use) => {
    const testDevices = testDeviceSet.testDevices as Record<string, TestDevice>;
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
