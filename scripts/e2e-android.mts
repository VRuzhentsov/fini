// Runs the project `device-support` with virtual Android test devices
// (ADR-0013 D8, D11). The same command locally and in CI:
// `npm run test:e2e:android [-- <playwright args>]`.
//
// On the host it builds the `android-phones-runner` image and runs this same
// script in it, with /dev/kvm and the repo mounted. The container has its own
// network, so its adb server does not touch the adb server of the host. In
// the container (FINI_E2E_CONTAINER_RUNNER=1) it runs Playwright. Phones start
// only when a test needs them (ADR-0013 D7).

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { E2eEnv, E2eProject } from '../specs/e2e/e2e-project.ts';
import { TestDevicePlatform } from '../specs/e2e/multi-device/test-device-platform.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const IMAGE = process.env.FINI_ANDROID_PHONES_IMAGE ?? 'fini-android-phones-runner';
const ENGINE = process.env.CONTAINER ?? 'podman';
// This runner can start virtual Linux and virtual Android test devices.
process.env[E2eEnv.VirtualPlatforms] ??= [TestDevicePlatform.Linux, TestDevicePlatform.Android].join(',');
const PASSED_ENV = [
  E2eEnv.TestDevices, E2eEnv.VirtualPlatforms, E2eEnv.Keep, 'FINI_E2E_ROOT', 'FINI_E2E_KEEP_PHONES',
  'FINI_E2E_ANDROID_APK', 'FINI_E2E_EMULATOR_GPU', 'FINI_E2E_RUN_ID', 'CI',
];

const args = process.argv.slice(2);

function run(command: string, commandArgs: string[]): number {
  const result = spawnSync(command, commandArgs, { stdio: 'inherit', cwd: REPO_ROOT });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

// Only test devices with the physical source (`a=linux@tcp:9224,b=android@tcp:9223`):
// no phone starts, so Playwright runs here and reaches the forwarded ports of
// the host.
const testDevices = process.env[E2eEnv.TestDevices] ?? '';
const physicalOnly = testDevices !== ''
  && testDevices.split(',').filter(Boolean).every((entry) => /@tcp:\d+$/.test(entry.trim()));

if (process.env.FINI_E2E_CONTAINER_RUNNER === '1' || physicalOnly) {
  process.exit(run('npx', [
    'playwright', 'test', '--config', 'specs/e2e/playwright.config.ts', `--project=${E2eProject.DeviceSupport}`, ...args,
  ]));
}

// CI keeps the image in GHCR (FINI_ANDROID_PHONES_CACHE_PREFIX), with the
// hash of the Dockerfile as the tag: the stage copies nothing from the repo,
// so only the Dockerfile changes it. A pull of about 6 GB is quicker than the
// build. Without the prefix (local runs) the layer cache of the engine does
// the same job.
// An image name must be lowercase; GitHub gives the owner as `VRuzhentsov`.
const cachePrefix = (process.env.FINI_ANDROID_PHONES_CACHE_PREFIX ?? '').toLowerCase();
const cacheImage = cachePrefix
  ? `${cachePrefix}-android-phones-runner-cache:${createHash('sha256').update(readFileSync(join(REPO_ROOT, 'Dockerfile'))).digest('hex')}`
  : '';
if (cacheImage && run(ENGINE, ['pull', cacheImage]) === 0) {
  console.log(`Using cached android-phones-runner image: ${cacheImage}`);
  const tagged = run(ENGINE, ['tag', cacheImage, IMAGE]);
  if (tagged !== 0) process.exit(tagged);
} else {
  const tags = cacheImage ? ['-t', IMAGE, '-t', cacheImage] : ['-t', IMAGE];
  const built = run(ENGINE, ['build', '--target', 'android-phones-runner', ...tags, '.']);
  if (built !== 0) process.exit(built);
  if (cacheImage && process.env.FINI_ANDROID_PHONES_CACHE_PUSH === '1') {
    // A failed push costs only the next run's build, so the lane goes on.
    if (run(ENGINE, ['push', cacheImage]) !== 0) console.warn(`Could not push ${cacheImage}; the lane goes on.`);
  }
}

const envArgs = PASSED_ENV.filter((name) => process.env[name] !== undefined).flatMap((name) => ['-e', name]);
process.exit(run(ENGINE, [
  // --shm-size: with the default 64 MB of /dev/shm the emulator crashes
  // (SIGSEGV) at start.
  'run', '--rm', '--device', '/dev/kvm', '--shm-size=2g',
  // `:Z` relabels the mount for SELinux, as ANDROID_BUILD_RUN in the Makefile does.
  '-v', `${REPO_ROOT}:/work:Z`, '-w', '/work', ...envArgs,
  IMAGE, 'node', 'scripts/e2e-android.mts', ...args,
]));
