import { mkdtempSync, rmSync } from 'fs';
import { spawnSync } from 'child_process';
import { join, dirname } from 'path';
import { tmpdir } from 'os';
import { fileURLToPath } from 'url';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..');

export interface CliClient {
  run<T = unknown>(args: string[]): T;
  close(): void;
}

export function createCliClient(): CliClient {
  const dbDir = mkdtempSync(join(tmpdir(), 'fini-e2e-'));
  const dbPath = join(dbDir, 'fini.db');
  const binary = process.env.FINI_BINARY ?? join(REPO_ROOT, 'src-tauri/target/debug/fini');

  function run<T = unknown>(args: string[]): T {
    const result = spawnSync(binary, ['--json', ...args], {
      env: { ...process.env, FINI_DB_PATH: dbPath, TZ: 'UTC' },
      encoding: 'utf8',
    });

    if (result.status !== 0) {
      throw new Error(result.stderr.trim() || `CLI command failed: ${args.join(' ')}`);
    }

    const stdout = result.stdout.trim();
    return stdout ? JSON.parse(stdout) as T : null as T;
  }

  return {
    run,
    close() {
      try {
        rmSync(dbDir, { recursive: true, force: true });
      } catch {}
    },
  };
}
