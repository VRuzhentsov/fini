// Android Emulator as a phone tool. All emulators on one host connect to the
// same netsimd, so they share one virtual Bluetooth medium. One AVD starts N
// times with `-read-only`.

import { spawn, execFile, type ChildProcess } from 'node:child_process';
import { mkdirSync, openSync } from 'node:fs';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { RUN_ROOT } from '../run-root.ts';
import { PhoneBootError, type PhoneTool } from './phone-tool.ts';

const run = promisify(execFile);

const FIRST_CONSOLE_PORT = 5554;
// Two phones boot at the same time with software rendering. On a CI runner one
// boot took more than 180 s.
const BOOT_TIMEOUT_MS = 360_000;
const START_INTERVAL_MS = 5_000;

export class AndroidEmulator implements PhoneTool {
  readonly name = 'emulator';
  private readonly processes: ChildProcess[] = [];

  constructor(
    private readonly avd = process.env.FINI_E2E_AVD ?? 'fini-e2e',
    // swiftshader_indirect stops the emulator after about 30 s on some hosts
    // (Bazzite); angle_indirect works there and in the container.
    private readonly gpu = process.env.FINI_E2E_EMULATOR_GPU ?? 'angle_indirect',
    private readonly emulatorBinary = `${process.env.ANDROID_HOME ?? ''}/emulator/emulator`,
  ) {}

  async up(count: number): Promise<string[]> {
    const serials: string[] = [];
    mkdirSync(RUN_ROOT, { recursive: true });
    for (let i = 0; i < count; i++) {
      const port = FIRST_CONSOLE_PORT + 2 * i;
      const log = openSync(join(RUN_ROOT, `emulator-${port}.log`), 'w');
      const child = spawn(this.emulatorBinary, [
        '-avd', this.avd, '-read-only', '-port', String(port),
        '-no-window', '-no-audio', '-no-snapshot', '-no-metrics', '-gpu', this.gpu,
      ], { stdio: ['ignore', log, log], detached: true });
      child.unref();
      this.processes.push(child);
      serials.push(`emulator-${port}`);
      // Two emulators that start in the same second on one read-only AVD
      // can stop or not boot.
      if (i < count - 1) await new Promise((resolve) => setTimeout(resolve, START_INTERVAL_MS));
    }
    await Promise.all(serials.map((serial) => this.waitForBoot(serial)));
    return serials;
  }

  async down(serials: string[]): Promise<void> {
    await Promise.all(serials.map((serial) => run('adb', ['-s', serial, 'emu', 'kill']).catch(() => undefined)));
  }

  private async waitForBoot(serial: string): Promise<void> {
    const end = Date.now() + BOOT_TIMEOUT_MS;
    while (Date.now() < end) {
      const booted = await run('adb', ['-s', serial, 'shell', 'getprop', 'sys.boot_completed'])
        .then(({ stdout }) => stdout.trim() === '1', () => false);
      // The package manager answers some time after boot_completed.
      const packagesReady = booted && await run('adb', ['-s', serial, 'shell', 'pm', 'path', 'android'])
        .then(({ stdout }) => stdout.includes('package:'), () => false);
      if (packagesReady) return;
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new PhoneBootError(`${serial} did not boot in ${BOOT_TIMEOUT_MS / 1000} s`);
  }
}
