// A test device that is the Fini debug app on a virtual phone of the phone
// group (ADR-0012 D2). Playwright `_android` finds the phone and runs the
// shell commands; the app actions go through the devtools bridge on TCP 9223,
// which `adb forward` makes available on a host port for each phone.

import { _android, type AndroidDevice } from '@playwright/test';
import type { PluginClient } from '@srsholmes/tauri-playwright';
import { execFile } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { promisify } from 'node:util';
import type { TestDeviceProvider } from '../test-device-set.ts';
import { REPO_ROOT } from '../run-root.ts';
import { connectOverTcp } from './physical-device.ts';

const run = promisify(execFile);

const PACKAGE = process.env.FINI_E2E_PACKAGE ?? 'com.fini.app.debug';
const ACTIVITY = `${PACKAGE}/com.fini.app.MainActivity`;
const DEFAULT_APK = join(REPO_ROOT, 'src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk');
/** The port of the devtools bridge in the app (`DEVTOOLS_ANDROID_TCP_PORT`, src-tauri/src/lib.rs). */
const APP_BRIDGE_PORT = 9223;
const FIRST_HOST_PORT = 9223;
// The first app start on a phone that has just booted can take minutes.
const CONNECT_TIMEOUT_MS = 180_000;
const APP_CHECK_INTERVAL_MS = 30_000;
const INSTALL_TIMEOUT_MS = 180_000;
const INSTALL_ATTEMPTS = 3;
const PERMISSIONS = ['BLUETOOTH_SCAN', 'BLUETOOTH_CONNECT', 'BLUETOOTH_ADVERTISE', 'POST_NOTIFICATIONS'];

export class AndroidPhoneProvider implements TestDeviceProvider {
  readonly kind = 'android-phone' as const;
  private device: AndroidDevice | null = null;
  private readonly hostPort: number;

  constructor(
    readonly slug: string,
    private readonly serial: string,
    phoneIndex: number,
    private readonly runDir: string,
  ) {
    this.hostPort = FIRST_HOST_PORT + 2 * phoneIndex;
  }

  /** Installs the APK, clears the app data, grants permissions and starts the app in front. */
  async prepare(): Promise<void> {
    // omitDriverInstall: by default Playwright installs its own driver app on
    // each phone. Only `shell` is used here, which works without it.
    const device = (await _android.devices({ omitDriverInstall: true })).find((item) => item.serial() === this.serial);
    if (!device) throw new Error(`phone ${this.serial} for test device "${this.slug}" is not in adb devices`);
    this.device = device;

    // `adb install`, not `device.installApk`: Playwright streams the APK in one
    // write, and with the 455 MB debug APK that write fails with EPIPE.
    await this.install(process.env.FINI_E2E_ANDROID_APK ?? DEFAULT_APK);
    await this.shell(`pm clear ${PACKAGE}`);
    for (const permission of PERMISSIONS) {
      await this.shell(`pm grant ${PACKAGE} android.permission.${permission}`);
    }
    await this.shell('cmd bluetooth_manager enable');
    // A webview in the background or behind the lock screen slows its timers,
    // and each `eval` then times out. Keep the screen on and unlocked, and the
    // app in front.
    await this.shell('svc power stayon true');
    await this.shell('input keyevent KEYCODE_WAKEUP');
    await this.shell('locksettings set-disabled true');
    await this.shell('wm dismiss-keyguard');
    await this.shell('logcat -b all -c');
    await this.shell(`am start -W -n ${ACTIVITY}`);
    await run('adb', ['-s', this.serial, 'forward', `tcp:${this.hostPort}`, `tcp:${APP_BRIDGE_PORT}`]);
  }

  async connect(): Promise<PluginClient> {
    // The bridge listens a short time after the activity starts.
    const end = Date.now() + CONNECT_TIMEOUT_MS;
    let lastError: unknown;
    let nextAppCheck = Date.now() + APP_CHECK_INTERVAL_MS;
    while (Date.now() < end) {
      try {
        return await connectOverTcp(this.slug, this.hostPort);
      } catch (error) {
        lastError = error;
        // On an emulator that has just booted, the first app start failed
        // once in two runs (the bridge never answered, cause not found).
        // Start the app again when its process is not there.
        if (Date.now() > nextAppCheck) {
          nextAppCheck = Date.now() + APP_CHECK_INTERVAL_MS;
          if ((await this.shell(`pidof ${PACKAGE}`)).trim() === '') {
            await this.shell(`am start -W -n ${ACTIVITY}`);
          }
        }
        await new Promise((resolve) => setTimeout(resolve, 1_000));
      }
    }
    throw lastError instanceof Error ? lastError : new Error(`test device "${this.slug}" did not connect`);
  }

  /** Saves the logcat of the phone in the run folder. The phone stays on for the next test file. */
  async dispose(): Promise<void> {
    if (!this.device) return;
    const logs = join(this.runDir, 'test-devices');
    mkdirSync(logs, { recursive: true });
    const { stdout } = await run('adb', ['-s', this.serial, 'logcat', '-b', 'all', '-d', '-v', 'threadtime,UTC'], {
      maxBuffer: 256 * 1024 * 1024,
    }).catch(() => ({ stdout: '' }));
    writeFileSync(join(logs, `${this.slug}.logcat.txt`), stdout);
    await run('adb', ['-s', this.serial, 'forward', '--remove', `tcp:${this.hostPort}`]).catch(() => undefined);
  }

  /** An install soon after boot can stop and never end, so each try has a time limit. */
  private async install(apk: string): Promise<void> {
    let lastError: unknown;
    for (let attempt = 1; attempt <= INSTALL_ATTEMPTS; attempt++) {
      try {
        await run('adb', ['-s', this.serial, 'install', '-r', '-g', apk], {
          maxBuffer: 16 * 1024 * 1024,
          timeout: INSTALL_TIMEOUT_MS,
        });
        return;
      } catch (error) {
        lastError = error;
      }
    }
    throw new Error(`install on ${this.serial} failed ${INSTALL_ATTEMPTS} times: ${String(lastError)}`);
  }

  private async shell(command: string): Promise<string> {
    return (await this.device!.shell(command)).toString();
  }
}
