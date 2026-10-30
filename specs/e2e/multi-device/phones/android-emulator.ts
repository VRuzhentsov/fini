// Android Emulator as a phone tool. All emulators on one host connect to the
// same netsimd, so they share one virtual Bluetooth medium. One AVD starts N
// times with `-read-only`.

import { spawn, execFile, type ChildProcess } from 'node:child_process';
import { promisify } from 'node:util';
import { PhoneBootError, type PhoneTool } from './phone-tool.ts';

const run = promisify(execFile);

const FIRST_CONSOLE_PORT = 5554;
const BOOT_TIMEOUT_MS = 180_000;

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
    for (let i = 0; i < count; i++) {
      const port = FIRST_CONSOLE_PORT + 2 * i;
      const child = spawn(this.emulatorBinary, [
        '-avd', this.avd, '-read-only', '-port', String(port),
        '-no-window', '-no-audio', '-no-snapshot', '-no-metrics', '-gpu', this.gpu,
      ], { stdio: 'ignore', detached: true });
      child.unref();
      this.processes.push(child);
      serials.push(`emulator-${port}`);
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
