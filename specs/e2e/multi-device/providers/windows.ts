// A virtual Windows test device: a `fini-app.exe` process. The harness starts
// it, gives it all its configuration through the environment, and talks to it
// over TCP on loopback (the devtools bridge has no unix socket on Windows).

import type { PluginClient } from '@srsholmes/tauri-playwright';
import { execFile, spawn, type ChildProcess } from 'node:child_process';
import { createSocket } from 'node:dgram';
import { appendFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { join } from 'node:path';
import type { TestDeviceProvider } from '../test-device-set.ts';
import { TestDevicePlatform, TestDeviceSource } from '../test-device-platform.ts';
import { REPO_ROOT } from '../run-root.ts';
import { connectOverTcp } from './tcp.ts';

const DEFAULT_APP_BINARY = join(REPO_ROOT, 'src-tauri/target/debug/fini-app.exe');
const DEFAULT_WAIT_SECS = 60;
/** A random start for each worker, so that parallel workers use different ports. */
const DEFAULT_BASE_PORT = 47_000 + Math.floor(Math.random() * 500);
/** The ports of one test device: devtools (TCP), discovery, Network endpoint (UDP). */
const PORTS_PER_TEST_DEVICE = 3;
/** The next block of a test device is this far up: clear of the blocks of 16 test devices. */
const PORT_STEP = PORTS_PER_TEST_DEVICE * 16;
const PORT_ATTEMPTS = 20;
/** How long a graceful close may take before `dispose` ends the process. */
const CLOSE_TIMEOUT_MS = 10_000;

export class WindowsProvider implements TestDeviceProvider {
  readonly platform = TestDevicePlatform.Windows;
  readonly source = TestDeviceSource.Virtual;
  private child: ChildProcess | null = null;
  private spawnError: Error | null = null;
  private readonly binaryPath: string;
  private readonly dataDir: string;
  private readonly logPath: string;
  private readonly index: number;
  private devtoolsPort = 0;
  private discoveryPort = 0;
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
    this.index = index;
    const waitSecs = Number.parseInt(process.env.FINI_E2E_TEST_DEVICE_WAIT_SECS ?? '', 10);
    this.waitMs = (Number.isFinite(waitSecs) && waitSecs > 0 ? waitSecs : DEFAULT_WAIT_SECS) * 1_000;
  }

  async prepare(): Promise<void> {
    mkdirSync(this.dataDir, { recursive: true });
    const base = await freePortBlock(DEFAULT_BASE_PORT + this.index * PORTS_PER_TEST_DEVICE);
    this.devtoolsPort = base;
    this.discoveryPort = base + 1;
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

  /**
   * Closes the app as a person does, so that its shutdown runs (`RunEvent::Exit`).
   * Windows has no SIGTERM: `taskkill` without `/F` sends WM_CLOSE to the
   * window. When the app has not stopped after `CLOSE_TIMEOUT_MS`, kill()
   * ends it at once.
   */
  async dispose(): Promise<void> {
    const child = this.child;
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const exited = new Promise<void>((resolve) => child.once('exit', () => resolve()));
    const pid = child.pid;
    // taskkill fails when the process has no window yet; then kill at once.
    const closeSent = pid !== undefined && await new Promise<boolean>((resolve) => {
      execFile('taskkill', ['/PID', String(pid)], (error) => resolve(error === null));
    });
    if (closeSent && await exitsWithin(exited, CLOSE_TIMEOUT_MS)) return;
    if (closeSent) {
      appendFileSync(this.logPath, `[${new Date().toISOString()}] close timed out after ${CLOSE_TIMEOUT_MS} ms; kill\n`);
    }
    child.kill();
    await exitsWithin(exited, 5_000);
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

/** True when `exited` resolves before `ms` ends. */
async function exitsWithin(exited: Promise<void>, ms: number): Promise<boolean> {
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<boolean>((resolve) => {
    timer = setTimeout(() => resolve(false), ms);
  });
  try {
    return await Promise.race([exited.then(() => true), timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * The first port of a block of `PORTS_PER_TEST_DEVICE` ports that no other
 * process uses now, from `start` up in steps of `PORT_STEP`. Another process
 * can still take a port before the app binds it, but a port that is in use
 * already is not given to the app.
 */
async function freePortBlock(start: number): Promise<number> {
  for (let attempt = 0; attempt < PORT_ATTEMPTS; attempt += 1) {
    const base = start + attempt * PORT_STEP;
    const ports = Array.from({ length: PORTS_PER_TEST_DEVICE }, (_, offset) => base + offset);
    const free = await Promise.all(ports.map(async (port) => await tcpPortFree(port) && await udpPortFree(port)));
    if (free.every(Boolean)) return base;
  }
  throw new Error(`no free block of ${PORTS_PER_TEST_DEVICE} ports from ${start} in ${PORT_ATTEMPTS} attempts`);
}

function tcpPortFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = createServer();
    server.once('error', () => resolve(false));
    server.listen({ port, host: '127.0.0.1', exclusive: true }, () => server.close(() => resolve(true)));
  });
}

function udpPortFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createSocket('udp4');
    socket.once('error', () => {
      socket.close();
      resolve(false);
    });
    socket.bind({ port, address: '0.0.0.0', exclusive: true }, () => socket.close(() => resolve(true)));
  });
}
