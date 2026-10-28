// PhoneGroup starts the virtual phones of a run, once, on the first request
// (ADR-0012 D7). It writes the serials to `android-phones.json`, so that a
// worker that Playwright starts again connects to the same phones and does not
// start new ones. `global-teardown.ts` stops them.

import { execFile } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { RUN_ROOT } from '../run-root.ts';
import { AndroidEmulator } from './android-emulator.ts';
import { Cuttlefish } from './cuttlefish.ts';
import type { PhoneTool } from './phone-tool.ts';

const run = promisify(execFile);

export const PHONE_GROUP_FILE = join(RUN_ROOT, 'android-phones.json');

interface PhoneGroupState {
  tool: string;
  serials: string[];
}

export class PhoneGroup {
  private static instance: PhoneGroup | null = null;
  private starting: Promise<string[]> | null = null;

  /** One group for each worker process. */
  static shared(): PhoneGroup {
    PhoneGroup.instance ??= new PhoneGroup();
    return PhoneGroup.instance;
  }

  /** Gives the serials of at least `count` phones, and starts them if they do not run. */
  serials(count: number): Promise<string[]> {
    this.starting ??= this.attachOrStart(count);
    return this.starting;
  }

  static tool(name = process.env.FINI_E2E_PHONE_TOOL ?? 'emulator'): PhoneTool {
    if (name === 'cuttlefish') return new Cuttlefish();
    if (name === 'emulator') return new AndroidEmulator();
    throw new Error(`unknown FINI_E2E_PHONE_TOOL "${name}" (use emulator or cuttlefish)`);
  }

  /** Stops the phones of the state file, if there is one. */
  static async stopRunning(): Promise<void> {
    const state = readState();
    if (!state) return;
    await PhoneGroup.tool(state.tool).down(state.serials);
    rmSync(PHONE_GROUP_FILE, { force: true });
  }

  private async attachOrStart(count: number): Promise<string[]> {
    const state = readState();
    if (state && state.serials.length >= count && (await allOnline(state.serials))) return state.serials;

    const tool = PhoneGroup.tool();
    const serials = await tool.up(count);
    mkdirSync(RUN_ROOT, { recursive: true });
    writeFileSync(PHONE_GROUP_FILE, JSON.stringify({ tool: tool.name, serials } satisfies PhoneGroupState, null, 2));
    return serials;
  }
}

function readState(): PhoneGroupState | null {
  if (!existsSync(PHONE_GROUP_FILE)) return null;
  return JSON.parse(readFileSync(PHONE_GROUP_FILE, 'utf8')) as PhoneGroupState;
}

async function allOnline(serials: string[]): Promise<boolean> {
  const states = await Promise.all(serials.map((serial) =>
    run('adb', ['-s', serial, 'get-state']).then(({ stdout }) => stdout.trim(), () => 'offline')));
  return states.every((state) => state === 'device');
}
