// The pull-request and release gate steps (ADR-0013 D9), for a developer's
// computer: the `make pr-gate-*` targets call them as `npm run gate:<step>`.
// Each step builds a stage of containers/linux/Containerfile with the
// container engine (see `engine()`; docs/container.md, "Container engine") and
// runs its command in it, with the repo mounted at /work. CI does the same in
// .github/actions/container-stage, and runs the same commands.
//
//   node scripts/ci-gate.mts <fe-unit|be-compile|be-unit|e2e-build-image|
//                             e2e-run|e2e-artifacts|e2e-cleanup>
//
// `e2e-run` in the `linux` `e2e` container (FINI_E2E_CONTAINER_RUNNER=1)
// builds the e2e binaries and runs scripts/e2e-runner.sh; CI calls it there.

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { E2eEnv } from '../specs/e2e/e2e-project.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const env = process.env;
const BE_FEATURES = 'cli-plane,ui-plane,desktop-updater';

const settings = {
  // The GHCR layer cache, for example `ghcr.io/vruzhentsov/fini-cache`. Empty: the local layer cache only.
  cache: env.FINI_CONTAINER_CACHE ?? '',
  cachePush: env.FINI_CONTAINER_CACHE_PUSH === '1',
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

function run(command: string, args: string[], { allowFailure = false, extraEnv = {} as Record<string, string> } = {}): number {
  const result = spawnSync(command, args, { stdio: 'inherit', cwd: REPO_ROOT, env: { ...env, ...extraEnv } });
  if (result.error) throw result.error;
  if (!allowFailure && result.status !== 0) process.exit(result.status ?? 1);
  return result.status ?? 1;
}

let chosenEngine = '';

const installed = (name: string) => spawnSync(name, ['--version'], { stdio: 'ignore' }).status === 0;

/**
 * The container engine: CONTAINER_ENGINE (podman|docker) when it is set, else
 * podman when it is installed, else docker. CI uses podman. CONTAINER_ENGINE
 * that is empty, `missing` (the value that the Makefile gives when it finds no
 * engine) or not installed stops the script, as containers/run.sh does.
 */
function engine(): string {
  if (chosenEngine) return chosenEngine;
  const name = env.CONTAINER_ENGINE ?? ['podman', 'docker'].find(installed) ?? '';
  if (name === '' || name === 'missing' || !installed(name)) {
    console.error(`No container engine ('${name}'). Install podman or docker, or set CONTAINER_ENGINE=podman|docker.`);
    process.exit(1);
  }
  chosenEngine = name;
  return chosenEngine;
}

/** Docker, or a docker command (`docker*`, as containers/run.sh finds it). All other engines are podman. */
const isDocker = () => basename(engine()).startsWith('docker');
const isPodman = () => !isDocker();

/**
 * Local name of a stage, the same name that .github/actions/container-stage gives.
 * A plain name, so that docker and podman both find it.
 */
const imageName = (stage: string) => `fini-linux-${stage}`;

/**
 * Builds a stage of containers/linux/Containerfile. With podman it uses the
 * flags of .github/actions/container-stage, and the GHCR layer cache when
 * FINI_CONTAINER_CACHE is set. With docker it is a plain `docker build` with
 * the local cache of docker.
 */
function buildStage(stage: string): string {
  const image = imageName(stage);
  const cache = settings.cache ? `${settings.cache}/linux` : '';
  const podmanArgs = isPodman()
    ? [
      '--layers',
      ...(cache ? ['--cache-from', cache] : []),
      ...(cache && settings.cachePush ? ['--cache-to', cache] : []),
      '--cache-ttl=168h', '--retry', '5', '--retry-delay', '5s',
    ]
    : [];
  run(engine(), [
    'build', ...podmanArgs,
    '-f', 'containers/linux/Containerfile', '--target', stage, '-t', image, '.',
  ]);
  return image;
}

/** A bind mount; with podman, `:z` gives it a shared SELinux label, as containers/run.sh does. */
const mount = (source: string, target: string) => ['-v', `${source}:${target}${isPodman() ? ':z' : ''}`];

/**
 * The named volumes of a run, as in containers/run.sh: the cargo registry,
 * npm and $HOME below /cache, and the node_modules and target directories of
 * the container apart from the ones of the host.
 */
const VOLUMES = [
  'fini-cargo:/cache/cargo', 'fini-npm:/cache/npm', 'fini-home:/cache/home',
  'fini-linux-node-modules:/work/node_modules',
  'fini-linux-target:/work/src-tauri/target',
  'fini-linux-broker-target:/work/ble-mock-broker/target',
];

/**
 * With docker, the command runs with the uid and gid of the user, so that the
 * files in the repo do not belong to root; the volumes get this owner first.
 * With rootless podman, root in the container is the user on the host.
 */
function userArgs(stage: string): string[] {
  if (!isDocker()) return [];
  const owner = `${process.getuid?.()}:${process.getgid?.()}`;
  run(engine(), [
    'run', '--rm', '--user', '0:0', ...VOLUMES.flatMap((volume) => ['-v', volume]),
    imageName(stage), 'chown', owner, ...VOLUMES.map((volume) => volume.split(':')[1]),
  ]);
  return ['--user', owner];
}

/** Runs `command` in a stage, with the repo mounted at /work. */
function runInStage(stage: string, command: string, { name = '', extraArgs = [] as string[] } = {}): void {
  run(engine(), [
    'run', '--rm', ...(name ? ['--name', name] : []), ...userArgs(stage),
    ...mount(REPO_ROOT, '/work'), '-w', '/work',
    ...VOLUMES.flatMap((volume) => ['-v', volume]),
    '-e', 'CARGO_HOME=/cache/cargo', '-e', 'npm_config_cache=/cache/npm', '-e', 'HOME=/cache/home',
    '-e', 'CARGO_INCREMENTAL=0', '-e', 'CARGO_PROFILE_DEV_DEBUG=line-tables-only',
    ...extraArgs,
    imageName(stage), 'bash', '-euo', 'pipefail', '-c', command,
  ]);
}

/** In the `e2e` container: builds the e2e binaries and runs scripts/e2e-runner.sh. */
function e2eInContainer(): void {
  mkdirSync(resultsDir, { recursive: true });
  const targetDir = env.CARGO_TARGET_DIR ?? join(REPO_ROOT, 'src-tauri', 'target');
  const brokerBinary = env.CARGO_TARGET_DIR
    ? join(targetDir, 'debug', 'ble-mock-broker')
    : join(REPO_ROOT, 'ble-mock-broker', 'target', 'debug', 'ble-mock-broker');
  run('node', [
    'scripts/with-devtools-capability.mts', 'npm', 'run', 'tauri', '--',
    'build', '--debug', '--features', 'ui-plane,devtools', '--no-bundle', '--', '--bin', 'fini-app',
  ]);
  run('cargo', ['build', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', 'fini', '--features', 'cli-plane']);
  run('cargo', ['build', '--manifest-path', 'ble-mock-broker/Cargo.toml']);
  run('sh', ['scripts/e2e-runner.sh'], {
    extraEnv: {
      ...(settings.testDevices ? { [E2eEnv.TestDevices]: settings.testDevices } : {}),
      FINI_E2E_CI_RUN_ID: settings.runId,
      FINI_E2E_RUN_ID: settings.runId,
      FINI_E2E_TEST_DEVICE_WAIT_SECS: settings.testDeviceWaitSecs,
      [E2eEnv.Keep]: settings.keep,
      FINI_E2E_SOCKET_DIR: '/var/run/fini-e2e',
      FINI_E2E_ROOT: join(resultsDir, 'fini-e2e-runs'),
      FINI_APP_BINARY: join(targetDir, 'debug', 'fini-app'),
      FINI_CLI_BINARY: join(targetDir, 'debug', 'fini'),
      FINI_BLE_MOCK_BROKER_BINARY: brokerBinary,
      FINI_E2E_CONTAINER_RUNNER: '1',
      TZ: 'UTC',
    },
  });
}

const steps: Record<string, () => unknown> = {
  'fe-unit': () => {
    buildStage('build');
    runInStage('build', 'npm ci && npm run test:unit');
  },

  'be-compile': () => {
    buildStage('build');
    runInStage('build', `cargo test --manifest-path src-tauri/Cargo.toml --no-run --features ${BE_FEATURES}`);
  },

  'be-unit': () => {
    buildStage('build');
    runInStage('build', `cargo test --manifest-path src-tauri/Cargo.toml --features ${BE_FEATURES}`);
  },

  'e2e-build-image': () => buildStage('e2e'),

  'e2e-run': () => {
    if (env.FINI_E2E_CONTAINER_RUNNER === '1') {
      e2eInContainer();
      return;
    }
    mkdirSync(resultsDir, { recursive: true });
    run(engine(), ['rm', '-f', runnerName], { allowFailure: true });
    runInStage('e2e', 'npm ci && node scripts/ci-gate.mts e2e-run', {
      name: runnerName,
      extraArgs: [
        // FINI_SCRATCH_DIR can be out of the repo, so the results get a mount of their own.
        ...mount(resultsDir, '/e2e-results'), '-e', 'FINI_E2E_CI_RESULTS_DIR=/e2e-results',
        '-e', 'FINI_E2E_CONTAINER_RUNNER=1',
        '-e', `FINI_E2E_CI_RUN_ID=${settings.runId}`,
        '-e', `FINI_E2E_CI_TEST_DEVICES=${settings.testDevices}`,
        '-e', `FINI_E2E_CI_TEST_DEVICE_WAIT_SECS=${settings.testDeviceWaitSecs}`,
        '-e', `FINI_E2E_KEEP=${settings.keep}`,
      ],
    });
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
