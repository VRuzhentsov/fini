// A test device that is a desktop `fini-app` process. The harness starts it,
// gives it all its configuration through the environment, and talks to it
// over a unix socket at a path that the harness chose.

import { PluginClient } from '@srsholmes/tauri-playwright';
import { spawn, type ChildProcess } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import type { TestDeviceProvider } from '../test-device-set.ts';
import { REPO_ROOT } from '../run-root.ts';
import { stopProcess } from './stop-process.ts';

const DEFAULT_APP_BINARY = join(REPO_ROOT, 'src-tauri/target/debug-e2e/debug/fini-app');
const DEFAULT_WAIT_SECS = 60;
const DEFAULT_BASE_DISCOVERY_PORT = 46_000 + Math.floor(Math.random() * 500);

/**
 * How desktop processes reach each other. 'network' is the real Network
 * channel. 'ble' turns the Network channel off (`FINI_DISCOVERY_DISABLED=1`)
 * and gives each process the `ble-gatt` mock radio. See
 * `specs/e2e/transports.md`.
 */
export type Transport = 'network' | 'ble';

/** What the desktop processes of one run share: the binary, folders and ports. */
export class DesktopRun {
  readonly binaryPath: string;
  readonly baseDiscoveryPort: number;
  readonly socketDir: string;
  readonly waitMs: number;

  constructor(
    readonly runDir: string,
    readonly slugs: string[],
    readonly transport: Transport,
  ) {
    this.binaryPath = process.env.FINI_APP_BINARY ?? DEFAULT_APP_BINARY;
    if (!existsSync(this.binaryPath)) throw new Error(`Fini GUI binary not found: ${this.binaryPath}`);
    const port = Number.parseInt(process.env.FINI_E2E_BASE_DISCOVERY_PORT ?? '', 10);
    this.baseDiscoveryPort = Number.isFinite(port) && port > 0 ? port : DEFAULT_BASE_DISCOVERY_PORT;
    const waitSecs = Number.parseInt(process.env.FINI_E2E_TEST_DEVICE_WAIT_SECS ?? '', 10);
    this.waitMs = (Number.isFinite(waitSecs) && waitSecs > 0 ? waitSecs : DEFAULT_WAIT_SECS) * 1_000;
    this.socketDir = join(runDir, 'sockets');
    mkdirSync(this.socketDir, { recursive: true });
  }

  /** One broker for all processes, clear of the discovery and ws ports. */
  get brokerPort(): number {
    return this.baseDiscoveryPort + this.slugs.length * 2 + 2000;
  }
}

export class DesktopProcessProvider implements TestDeviceProvider {
  readonly kind = 'desktop-process' as const;
  private child: ChildProcess | null = null;
  private spawnError: Error | null = null;
  private readonly dataDir: string;
  private readonly logPath: string;
  private readonly socketPath: string;
  private readonly discoveryPort: number;

  constructor(
    readonly slug: string,
    private readonly index: number,
    private readonly run: DesktopRun,
  ) {
    const testDeviceDir = join(run.runDir, 'test-devices');
    this.dataDir = join(testDeviceDir, `${slug}-data`);
    this.logPath = join(testDeviceDir, `${slug}.log`);
    this.socketPath = join(run.socketDir, `${slug}.sock`);
    this.discoveryPort = run.baseDiscoveryPort + index * 2;
  }

  async prepare(): Promise<void> {
    mkdirSync(this.dataDir, { recursive: true });
    rmSync(this.socketPath, { force: true });
    const peerPorts = this.run.slugs.map((_, i) => String(this.run.baseDiscoveryPort + i * 2)).join(',');

    const child = spawn(this.run.binaryPath, [], {
      env: {
        ...process.env,
        FINI_TEST_DEVICE_SLUG: this.slug,
        FINI_APP_DATA_DIR: this.dataDir,
        FINI_DISCOVERY_PEER_PORTS: peerPorts,
        FINI_DISCOVERY_PORT: String(this.discoveryPort),
        FINI_SPACE_SYNC_WS_PORT: String(this.discoveryPort + 1),
        TAURI_PLAYWRIGHT_SOCKET: this.socketPath,
        // The WebKitGTK DMABUF renderer paints nothing on some GPUs (it logs
        // "Failed to create GBM buffer"), and then each `eval` times out.
        WEBKIT_DISABLE_DMABUF_RENDERER: '1',
        HOSTNAME: this.slug,
        TZ: 'UTC',
        XDG_DATA_HOME: this.dataDir,
        ...this.transportEnv(),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    this.child = child;
    appendFileSync(this.logPath, `[${new Date().toISOString()}] start test device ${this.slug}\n`);
    child.stdout.on('data', (chunk) => appendFileSync(this.logPath, chunk));
    child.stderr.on('data', (chunk) => appendFileSync(this.logPath, chunk));
    child.once('error', (error) => {
      this.spawnError = error;
      appendFileSync(this.logPath, `[${new Date().toISOString()}] spawn error: ${error.message}\n`);
    });
    await this.waitForSocket();
  }

  async connect(): Promise<PluginClient> {
    const client = new PluginClient(this.socketPath);
    await client.connect();
    const ping = await client.send({ type: 'ping' });
    if (!ping.ok) throw new Error(this.debugMessage('Plugin ping failed'));
    return client;
  }

  async dispose(): Promise<void> {
    if (this.child) await stopProcess(this.child);
  }

  private transportEnv(): Record<string, string> {
    if (this.run.transport !== 'ble') return {};
    return {
      FINI_DISCOVERY_DISABLED: '1',
      FINI_BLE_MOCK_BROKER: `127.0.0.1:${this.run.brokerPort}`,
      // `ble.rs` does not start on the mock broker without it. Stable for
      // each position, so no step between the processes is necessary.
      FINI_LOCAL_BLUETOOTH_ADDRESS: fakeBluetoothAddress(this.index),
    };
  }

  private async waitForSocket(): Promise<void> {
    const end = Date.now() + this.run.waitMs;
    while (Date.now() < end) {
      if (this.spawnError) throw new Error(this.debugMessage('Start failed before the socket was made'));
      if (existsSync(this.socketPath)) return;
      if (this.child && (this.child.exitCode !== null || this.child.signalCode !== null)) {
        throw new Error(this.debugMessage('Process stopped before the socket was made'));
      }
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    throw new Error(this.debugMessage(`Socket not made in ${this.run.waitMs} ms`));
  }

  private debugMessage(headline: string): string {
    const tail = existsSync(this.logPath)
      ? readFileSync(this.logPath, 'utf8').trim().split(/\r?\n/).slice(-40).join('\n')
      : '(log file missing)';
    return [
      headline,
      `slug: ${this.slug}`,
      `binary: ${this.run.binaryPath}`,
      `dataDir: ${this.dataDir}`,
      `socket: ${this.socketPath}`,
      `discoveryPort: ${this.discoveryPort}`,
      `logPath: ${this.logPath}`,
      this.spawnError ? `spawnError: ${this.spawnError.message}` : null,
      `exitCode: ${this.child?.exitCode ?? 'running'}`,
      'logTail:',
      tail,
    ].filter(Boolean).join('\n');
  }
}

/** The fake Bluetooth address of the desktop process at `index` in the `ble` lane. */
export function fakeBluetoothAddress(index: number): string {
  return `AA:BB:CC:00:00:${(index + 1).toString(16).padStart(2, '0').toUpperCase()}`;
}
