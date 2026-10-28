// A test device that is an app that already runs on a physical device, for
// example a phone through `adb forward`. The harness only connects to it over
// TCP: it does not start, configure or stop it. Its pairs and data are what
// the device has.

import { PluginClient } from '@srsholmes/tauri-playwright';
import type { TestDeviceProvider } from '../test-device-set.ts';

export class PhysicalDeviceProvider implements TestDeviceProvider {
  readonly kind = 'physical-device' as const;

  constructor(readonly slug: string, private readonly port: number) {}

  async prepare(): Promise<void> {
    // The person who started the app prepared it.
  }

  async connect(): Promise<PluginClient> {
    return connectOverTcp(this.slug, this.port);
  }

  async dispose(): Promise<void> {
    // The harness did not start this app, so it does not stop it.
  }
}

/** Connects to the devtools bridge of an app on TCP `port` and checks it answers. */
export async function connectOverTcp(slug: string, port: number): Promise<PluginClient> {
  const client = new PluginClient(undefined, port);
  await client.connect();
  const ping = await client.send({ type: 'ping' });
  if (!ping.ok) {
    throw new Error(
      `Test device "${slug}" did not answer ping on tcp:${port}. Is the app running, built with the `
        + `\`devtools\` feature, and the port forwarded (\`adb forward tcp:${port} tcp:9223\`)?`,
    );
  }
  return client;
}
