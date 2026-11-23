// Runs the projects `ui` and `multi-device` on the display of this computer,
// with the binaries of `npm run build:desktop:e2e`. `npm run test:e2e`
// (`make e2e-headed`) builds them first, then calls this script:
//
//   node scripts/e2e-headed.mts [playwright args...]
//
// The run folder is FINI_E2E_ROOT, else $FINI_SCRATCH_DIR/fini-e2e-headed,
// else tmp/fini-e2e-headed. An empty variable counts as not set.

import { spawnSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { E2eProject } from '../specs/e2e/e2e-project.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const env = process.env;
const binaryDir = join(REPO_ROOT, 'src-tauri', 'target', 'debug-e2e', 'debug');
const runRoot = env.FINI_E2E_ROOT || join(env.FINI_SCRATCH_DIR || join(REPO_ROOT, 'tmp'), 'fini-e2e-headed');

mkdirSync(runRoot, { recursive: true });
const result = spawnSync('npx', [
  'playwright', 'test', '--config', 'specs/e2e/playwright.config.ts',
  '--project', E2eProject.Ui, '--project', E2eProject.MultiDevice, ...process.argv.slice(2),
], {
  stdio: 'inherit',
  cwd: REPO_ROOT,
  // On Windows, npx is a .cmd file, which only a shell can start.
  shell: process.platform === 'win32',
  env: {
    ...env,
    FINI_E2E_ROOT: runRoot,
    FINI_APP_BINARY: join(binaryDir, 'fini-app'),
    FINI_CLI_BINARY: join(binaryDir, 'fini'),
    FINI_BLE_MOCK_BROKER_BINARY: join(binaryDir, 'ble-mock-broker'),
    TZ: 'UTC',
  },
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
