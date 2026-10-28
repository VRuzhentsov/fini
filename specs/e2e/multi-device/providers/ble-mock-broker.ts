// The shared mock radio of the `ble` lane: one `ble-mock-broker` process for
// each worker, started before the desktop processes. See ble-gatt
// `docs/adr/0004-mock-broker-for-cross-process-e2e.md`.

import { spawn, type ChildProcess } from 'node:child_process';
import { appendFileSync, existsSync } from 'node:fs';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import { REPO_ROOT } from '../run-root.ts';
import { stopProcess } from './stop-process.ts';

const DEFAULT_BINARY = join(REPO_ROOT, 'src-tauri/target/debug-e2e/debug/ble-mock-broker');
const READY_TIMEOUT_MS = 60_000;

export class BleMockBroker {
  private child: ChildProcess | null = null;
  private readonly logPath: string;

  constructor(runDir: string, readonly port: number) {
    this.logPath = join(runDir, 'ble-mock-broker.log');
  }

  async start(): Promise<void> {
    const binary = process.env.FINI_BLE_MOCK_BROKER_BINARY ?? DEFAULT_BINARY;
    if (!existsSync(binary)) throw new Error(`ble-mock-broker binary not found: ${binary}`);
    const child = spawn(binary, [], {
      env: { ...process.env, FINI_BLE_MOCK_BROKER_LISTEN: `127.0.0.1:${this.port}` },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    this.child = child;
    appendFileSync(this.logPath, `[${new Date().toISOString()}] start ble-mock-broker on 127.0.0.1:${this.port}\n`);
    child.stdout.on('data', (chunk) => appendFileSync(this.logPath, chunk));
    child.stderr.on('data', (chunk) => appendFileSync(this.logPath, chunk));
    await this.waitForReady();
  }

  async stop(): Promise<void> {
    if (this.child) await stopProcess(this.child).catch(() => undefined);
  }

  private async waitForReady(): Promise<void> {
    const end = Date.now() + READY_TIMEOUT_MS;
    while (Date.now() < end) {
      if (this.child && (this.child.exitCode !== null || this.child.signalCode !== null)) {
        throw new Error(`ble-mock-broker stopped before it was ready; see ${this.logPath}`);
      }
      const listening = await new Promise<boolean>((resolve) => {
        const socket = createConnection({ host: '127.0.0.1', port: this.port }, () => {
          socket.destroy();
          resolve(true);
        });
        socket.once('error', () => {
          socket.destroy();
          resolve(false);
        });
      });
      if (listening) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error(`ble-mock-broker did not listen on 127.0.0.1:${this.port} in ${READY_TIMEOUT_MS} ms; see ${this.logPath}`);
  }
}
