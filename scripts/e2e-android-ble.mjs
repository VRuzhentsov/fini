// Runs the android-ble e2e lane (ADR-0013 D8). The same command locally and
// in CI: `npm run test:e2e:android-ble [-- <playwright args>]`.
//
// On the host it builds the `android-phones-runner` image and runs this same
// script in it, with /dev/kvm and the repo mounted. The container has its own
// network, so its adb server does not touch the adb server of the host. In
// the container (FINI_E2E_CONTAINER_RUNNER=1) it runs Playwright. Phones start
// only when a test needs them (ADR-0013 D7).

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const IMAGE = process.env.FINI_ANDROID_PHONES_IMAGE ?? 'fini-android-phones-runner';
const ENGINE = process.env.CONTAINER ?? 'podman';
const PASSED_ENV = [
  'FINI_E2E_TEST_DEVICES', 'FINI_E2E_ROOT', 'FINI_E2E_KEEP', 'FINI_E2E_KEEP_PHONES',
  'FINI_E2E_ANDROID_APK', 'FINI_E2E_EMULATOR_GPU', 'FINI_E2E_RUN_ID', 'CI',
];

const args = process.argv.slice(2);

function run(command, commandArgs) {
  const result = spawnSync(command, commandArgs, { stdio: 'inherit', cwd: REPO_ROOT });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

// Physical devices only (`a=tcp:9224,b=tcp:9223`): no phone starts, so
// Playwright runs here and reaches the forwarded ports of the host.
const physicalOnly = (process.env.FINI_E2E_TEST_DEVICES ?? '')
  .split(',').filter(Boolean).every((entry) => /=tcp:\d+$/.test(entry.trim()))
  && Boolean(process.env.FINI_E2E_TEST_DEVICES);

if (process.env.FINI_E2E_CONTAINER_RUNNER === '1' || physicalOnly) {
  process.exit(run('npx', ['playwright', 'test', '--config', 'specs/e2e/playwright.config.ts', '--project=android-ble', ...args]));
}

const built = run(ENGINE, ['build', '--target', 'android-phones-runner', '-t', IMAGE, '.']);
if (built !== 0) process.exit(built);

const envArgs = PASSED_ENV.filter((name) => process.env[name] !== undefined).flatMap((name) => ['-e', name]);
process.exit(run(ENGINE, [
  // --shm-size: with the default 64 MB of /dev/shm the emulator crashes
  // (SIGSEGV) at start.
  'run', '--rm', '--device', '/dev/kvm', '--shm-size=2g',
  // `:Z` relabels the mount for SELinux, as ANDROID_BUILD_RUN in the Makefile does.
  '-v', `${REPO_ROOT}:/work:Z`, '-w', '/work', ...envArgs,
  IMAGE, 'node', 'scripts/e2e-android-ble.mjs', ...args,
]));
