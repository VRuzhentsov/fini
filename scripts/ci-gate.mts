// The pull-request and release gate steps (ADR-0013 D9). CI calls them as
// `npm run gate:<step>`; the `make pr-gate-*` targets call the same scripts.
// Each step reads the same environment variables that the Makefile set before.
//
//   node scripts/ci-gate.mts <fe-unit|be-cache-key|be-compile|be-unit|
//                             e2e-cache-key|e2e-build-image|e2e-run|
//                             e2e-artifacts|e2e-cleanup>

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { E2eEnv } from '../specs/e2e/e2e-project.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const env = process.env;

const BE_CACHE_INPUTS = [
  'Dockerfile', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/src',
  'src-tauri/migrations', 'src-tauri/patches', 'src-tauri/capabilities', 'src-tauri/icons',
  'src-tauri/tauri.conf.json',
];
const E2E_CACHE_INPUTS = [
  'Dockerfile', 'package.json', 'package-lock.json', 'tsconfig*.json', 'index.html', 'scripts/e2e-runner.sh',
  'vite.config.ts', 'src', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/src',
  'src-tauri/migrations', 'src-tauri/patches', 'src-tauri/capabilities', 'src-tauri/devtools-capabilities',
  'src-tauri/icons', 'src-tauri/tauri.conf.json', 'specs/e2e',
];

const settings = {
  beCompileImage: env.FINI_BE_COMPILE_IMAGE ?? 'fini-be-compile-ci',
  beUnitImage: env.FINI_BE_UNIT_IMAGE ?? 'fini-be-unit-test',
  beCachePrefix: env.FINI_BE_CACHE_IMAGE_PREFIX ?? '',
  beCachePush: env.FINI_BE_CACHE_PUSH === '1',
  devRunnerImage: env.FINI_DEV_RUNNER_IMAGE ?? 'fini-dev-runner-ci',
  e2eCachePrefix: env.FINI_E2E_CACHE_IMAGE_PREFIX ?? '',
  e2eCachePush: env.FINI_E2E_CACHE_PUSH === '1',
  runId: env.FINI_E2E_CI_RUN_ID ?? 'pr-gate',
  // Empty: each spec runs on its own test devices (`testDevicePlatforms`).
  testDevices: env.FINI_E2E_CI_TEST_DEVICES ?? '',
  testDeviceWaitSecs: env.FINI_E2E_CI_TEST_DEVICE_WAIT_SECS ?? '180',
  keep: env.FINI_E2E_KEEP ?? '',
};
const scratchDir = env.FINI_SCRATCH_DIR ?? join(REPO_ROOT, 'tmp');
const runDir = env.FINI_E2E_CI_RUN_DIR ?? join(scratchDir, `fini-e2e-${settings.runId}`);
const resultsDir = env.FINI_E2E_CI_RESULTS_DIR ?? join(runDir, 'test-results');
const runnerName = `fini-${settings.runId}-runner`;

/** The container engine: CONTAINER=docker|podman, or the first one found (docker first, as before). */
function engine(): string {
  const chosen = env.CONTAINER ?? 'auto';
  if (chosen !== 'auto') return chosen;
  for (const candidate of ['docker', 'podman']) {
    if (spawnSync(candidate, ['--version'], { stdio: 'ignore' }).status === 0) return candidate;
  }
  throw new Error('No container engine found. Install Docker or Podman, or set CONTAINER=docker|podman.');
}

function run(command: string, args: string[], { allowFailure = false } = {}): number {
  const result = spawnSync(command, args, { stdio: 'inherit', cwd: REPO_ROOT });
  if (result.error) throw result.error;
  if (!allowFailure && result.status !== 0) process.exit(result.status ?? 1);
  return result.status ?? 1;
}

/** sha256 of the tracked and not-ignored untracked files under `paths`, as the Makefile computed it. */
function cacheKey(paths: string[]): string {
  const list = (args: string[]) => spawnSync('git', ['ls-files', '-z', ...args, '--', ...paths], { cwd: REPO_ROOT, encoding: 'utf8' })
    .stdout.split('\0').filter(Boolean);
  const files = [...new Set([...list([]), ...list(['--others', '--exclude-standard'])])].sort();
  const lines = files
    .filter((file) => existsSync(join(REPO_ROOT, file)) && statSync(join(REPO_ROOT, file)).isFile())
    .map((file) => `${createHash('sha256').update(readFileSync(join(REPO_ROOT, file))).digest('hex')}  ${file}\n`)
    .join('');
  return createHash('sha256').update(lines).digest('hex');
}

interface CachedBuild {
  target: string;
  image: string;
  prefix: string;
  cacheName: string;
  key: string;
  label: string;
  push: boolean;
  extraArgs?: string[];
}

/** Pulls the cached image for `key` or builds `target`, and pushes the new cache image if allowed. */
function cachedBuild({ target, image, prefix, cacheName, key, label, push, extraArgs = [] }: CachedBuild): void {
  const cacheImage = prefix ? `${prefix}-${cacheName}:${key}` : '';
  const containerEngine = engine();
  if (cacheImage && run(containerEngine, ['pull', cacheImage], { allowFailure: true }) === 0) {
    console.log(`Using cached ${target} image: ${cacheImage}`);
    run(containerEngine, ['tag', cacheImage, image]);
    return;
  }
  console.log(`Building ${target} image for cache key: ${key}`);
  const tags = cacheImage ? ['-t', image, '-t', cacheImage] : ['-t', image];
  run(containerEngine, ['build', '--target', target, ...extraArgs, '--label', `${label}=${key}`, ...tags, '.']);
  if (cacheImage && push) run(containerEngine, ['push', cacheImage]);
}

const steps: Record<string, () => unknown> = {
  'fe-unit': () => run(engine(), ['build', '--target', 'fe-unit-test', '-t', 'fini-fe-unit-test', '.']),

  'be-cache-key': () => console.log(cacheKey(BE_CACHE_INPUTS)),

  'be-compile': () => cachedBuild({
    target: 'be-test-compile',
    image: settings.beCompileImage,
    prefix: settings.beCachePrefix,
    cacheName: 'be-compile-cache',
    key: cacheKey(BE_CACHE_INPUTS),
    label: 'fini.be.cache-key',
    push: settings.beCachePush,
    extraArgs: ['--build-arg', 'BUILDKIT_INLINE_CACHE=1'],
  }),

  'be-unit': () => {
    steps['be-compile']();
    run(engine(), ['build', '--target', 'be-unit-test', '--cache-from', settings.beCompileImage, '-t', settings.beUnitImage, '.']);
  },

  'e2e-cache-key': () => console.log(cacheKey(E2E_CACHE_INPUTS)),

  'e2e-build-image': () => cachedBuild({
    target: 'dev-runner',
    image: settings.devRunnerImage,
    prefix: settings.e2eCachePrefix,
    cacheName: 'dev-runner-cache',
    key: cacheKey(E2E_CACHE_INPUTS),
    label: 'fini.e2e.cache-key',
    push: settings.e2eCachePush,
  }),

  'e2e-run': () => {
    mkdirSync(resultsDir, { recursive: true });
    const containerEngine = engine();
    run(containerEngine, ['rm', '-f', runnerName], { allowFailure: true });
    run(containerEngine, [
      'run', '--rm', '--name', runnerName,
      ...(settings.testDevices ? ['-e', `${E2eEnv.TestDevices}=${settings.testDevices}`] : []),
      '-e', `FINI_E2E_CI_RUN_ID=${settings.runId}`,
      '-e', `FINI_E2E_RUN_ID=${settings.runId}`,
      '-e', `FINI_E2E_TEST_DEVICE_WAIT_SECS=${settings.testDeviceWaitSecs}`,
      '-e', `${E2eEnv.Keep}=${settings.keep}`,
      '-v', `${resultsDir}:/app/test-results:Z`,
      settings.devRunnerImage,
    ]);
  },

  'e2e-artifacts': () => {
    const logs = join(resultsDir, 'fini-e2e-runs', settings.runId, 'test-devices');
    console.log('\n===== test device logs =====');
    if (!existsSync(logs)) {
      console.log(`No test device log directory found: ${logs}`);
      return;
    }
    for (const name of readdirSync(logs).filter((file) => file.endsWith('.log'))) {
      console.log(`\n--- ${join(logs, name)} ---`);
      console.log(readFileSync(join(logs, name), 'utf8').split('\n').slice(-200).join('\n'));
    }
  },

  'e2e-cleanup': () => run(engine(), ['rm', '-f', runnerName], { allowFailure: true }),
};

const step = steps[process.argv[2]];
if (!step) {
  console.error(`usage: ci-gate.mts <${Object.keys(steps).join('|')}>`);
  process.exit(2);
}
step();
