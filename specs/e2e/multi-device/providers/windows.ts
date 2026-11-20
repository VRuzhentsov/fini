// A virtual Windows test device: a `fini-app.exe` process. The harness starts
// it, gives it all its configuration through the environment, and talks to it
// over TCP on loopback (the devtools bridge has no unix socket on Windows).

import type { PluginClient } from '@srsholmes/tauri-playwright';
import { spawn, type ChildProcess } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import type { TestDeviceProvider } from '../test-device-set.ts';
import { TestDevicePlatform, TestDeviceSource } from '../test-device-platform.ts';
import { REPO_ROOT } from '../run-root.ts';
import { connectOverTcp } from './tcp.ts';

const DEFAULT_APP_BINARY = join(REPO_ROOT, 'src-tauri/target/debug/fini-app.exe');
const DEFAULT_WAIT_SECS = 60;
const DEFAULT_BASE_PORT = 47_000 + Math.floor(Math.random() * 500);

export class WindowsProvider implements TestDeviceProvider {
  readonly platform = TestDevicePlatform.Windows;
  readonly source = TestDeviceSource.Virtual;
  private child: ChildProcess | null = null;
  private spawnError: Error | null = null;
  private readonly binaryPath: string;
  private readonly dataDir: string;
  private readonly logPath: string;
  private readonly devtoolsPort: number;
  private readonly discoveryPort: number;
  private readonly waitMs: number;

  constructor(
    readonly slug: string,
    index: number,
    runDir: string,
  ) {
    this.binaryPath = process.env.FINI_APP_BINARY ?? DEFAULT_APP_BINARY;
    if (!existsSync(this.binaryPath)) throw new Error(`Fini GUI binary not found: ${this.binaryPath}`);
    const testDeviceDir = join(runDir, 'test-devices');
    this.dataDir = join(testDeviceDir, `${slug}-data`);
    this.logPath = join(testDeviceDir, `${slug}.log`);
    const base = DEFAULT_BASE_PORT + index * 3;
    this.devtoolsPort = base;
    this.discoveryPort = base + 1;
    const waitSecs = Number.parseInt(process.env.FINI_E2E_TEST_DEVICE_WAIT_SECS ?? '', 10);
    this.waitMs = (Number.isFinite(waitSecs) && waitSecs > 0 ? waitSecs : DEFAULT_WAIT_SECS) * 1_000;
  }

  async prepare(): Promise<void> {
    mkdirSync(this.dataDir, { recursive: true });
    const child = spawn(this.binaryPath, [], {
      env: {
        ...process.env,
        FINI_TEST_DEVICE_SLUG: this.slug,
        FINI_APP_DATA_DIR: this.dataDir,
        FINI_DEVTOOLS_TCP_PORT: String(this.devtoolsPort),
        FINI_DISCOVERY_PORT: String(this.discoveryPort),
        FINI_SPACE_SYNC_WS_PORT: String(this.discoveryPort + 1),
        // WebView2 keeps its own data apart from an installed Fini.
        WEBVIEW2_USER_DATA_FOLDER: join(this.dataDir, 'webview2'),
        TZ: 'UTC',
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
  }

  /** Tries the TCP bridge until it answers, or the process stops, or the time ends. */
  async connect(): Promise<PluginClient> {
    const end = Date.now() + this.waitMs;
    let lastError: unknown;
    while (Date.now() < end) {
      if (this.spawnError) throw new Error(this.debugMessage('Start failed'));
      if (this.child && (this.child.exitCode !== null || this.child.signalCode !== null)) {
        throw new Error(this.debugMessage('Process stopped before the bridge answered'));
      }
      try {
        return await connectOverTcp(this.slug, this.devtoolsPort);
      } catch (error) {
        lastError = error;
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    const reason = lastError instanceof Error ? ` Last error: ${lastError.message}` : '';
    throw new Error(this.debugMessage(`Bridge did not answer in ${this.waitMs} ms.${reason}`));
  }

  async dispose(): Promise<void> {
    const child = this.child;
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    // Windows has no SIGTERM: kill() ends the process at once.
    await new Promise<void>((resolve) => {
      child.once('exit', () => resolve());
      child.kill();
      setTimeout(resolve, 5_000);
    });
  }

  private debugMessage(headline: string): string {
    const tail = existsSync(this.logPath)
      ? readFileSync(this.logPath, 'utf8').trim().split(/\r?\n/).slice(-40).join('\n')
      : '(log file missing)';
    return [
      headline,
      `slug: ${this.slug}`,
      `binary: ${this.binaryPath}`,
      `dataDir: ${this.dataDir}`,
      `devtoolsPort: ${this.devtoolsPort}`,
      `logPath: ${this.logPath}`,
      this.spawnError ? `spawnError: ${this.spawnError.message}` : null,
      `exitCode: ${this.child?.exitCode ?? 'running'}`,
      'logTail:',
      tail,
    ].filter(Boolean).join('\n');
  }
}
