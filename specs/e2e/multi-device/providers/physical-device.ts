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

const PING_TIMEOUT_MS = 5_000;

/**
 * Connects to the devtools bridge of an app on TCP `port` and checks it answers.
 * `adb forward` accepts the connection also when the app does not listen yet,
 * and then the ping gets no answer; so the ping has a time limit.
 */
export async function connectOverTcp(slug: string, port: number): Promise<PluginClient> {
  const client = new PluginClient(undefined, port);
  let timer: NodeJS.Timeout | undefined;
  const timeLimit = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`no answer on tcp:${port} in ${PING_TIMEOUT_MS} ms`)), PING_TIMEOUT_MS);
  });
  const attempt = client.connect().then(() => client.send({ type: 'ping' }));
  // After the time limit, this attempt can still fail (the socket closes).
  // Nobody waits for it then, so catch it here.
  attempt.catch(() => undefined);
  const ping = await Promise.race([attempt, timeLimit]).catch((error: unknown) => {
    try {
      client.disconnect();
    } catch {
      // The socket did not open.
    }
    throw error;
  }).finally(() => clearTimeout(timer));
  if (!ping.ok) {
    throw new Error(
      `Test device "${slug}" did not answer ping on tcp:${port}. Is the app running, built with the `
        + `\`devtools\` feature, and the port forwarded (\`adb forward tcp:${port} tcp:9223\`)?`,
    );
  }
  return client;
}
