// AOSP Cuttlefish as a phone tool. One `launch_cvd --num_instances=N` starts N
// phones that share one RootCanal Bluetooth medium. Instance i listens for
// adb on port 6520 + i.

import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { PhoneBootError, type PhoneTool } from './phone-tool';

const run = promisify(execFile);

const FIRST_ADB_PORT = 6520;
const BOOT_TIMEOUT_MS = 300_000;

export class Cuttlefish implements PhoneTool {
  readonly name = 'cuttlefish';

  /** `imagesDir` holds the fetched host package and phone images (`cvd fetch`). */
  constructor(private readonly imagesDir = process.env.FINI_E2E_CUTTLEFISH_DIR ?? '/cuttlefish') {}

  async up(count: number): Promise<string[]> {
    const env = { ...process.env, HOME: this.imagesDir };
    await run(`${this.imagesDir}/bin/launch_cvd`, [
      '--daemon', `--num_instances=${count}`, '--report_anonymous_usage_stats=n',
      '--cpus=2', '--memory_mb=3072',
    ], { env, cwd: this.imagesDir, timeout: BOOT_TIMEOUT_MS, maxBuffer: 64 * 1024 * 1024 })
      .catch((error: Error) => { throw new PhoneBootError(`launch_cvd failed: ${error.message}`); });

    const serials = Array.from({ length: count }, (_, i) => `0.0.0.0:${FIRST_ADB_PORT + i}`);
    await Promise.all(serials.map((serial) => this.waitForBoot(serial)));
    return serials;
  }

  async down(): Promise<void> {
    const env = { ...process.env, HOME: this.imagesDir };
    await run(`${this.imagesDir}/bin/stop_cvd`, [], { env, cwd: this.imagesDir }).catch(() => undefined);
  }

  private async waitForBoot(serial: string): Promise<void> {
    const end = Date.now() + BOOT_TIMEOUT_MS;
    while (Date.now() < end) {
      await run('adb', ['connect', serial]).catch(() => undefined);
      const booted = await run('adb', ['-s', serial, 'shell', 'getprop', 'sys.boot_completed'])
        .then(({ stdout }) => stdout.trim() === '1', () => false);
      if (booted) return;
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new PhoneBootError(`${serial} did not boot in ${BOOT_TIMEOUT_MS / 1000} s`);
  }
}
