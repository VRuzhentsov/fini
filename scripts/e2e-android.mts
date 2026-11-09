// Runs the project `device-support` with virtual Android test devices
// (ADR-0013 D8, D11). The same command locally and in CI:
// `npm run test:e2e:android [-- <playwright args>]`.
//
// On the host it builds the `android` `e2e` stage
// (containers/android/Containerfile) and runs this same script in it, with
// /dev/kvm and the repo mounted. The container has its own network, so its
// adb server does not touch the adb server of the host. In the container
// (FINI_E2E_CONTAINER_RUNNER=1) it runs Playwright; CI starts it there with
// .github/actions/container-stage. Phones start only when a test needs them
// (ADR-0013 D7).

import { spawnSync } from 'node:child_process';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { E2eEnv, E2eProject } from '../specs/e2e/e2e-project.ts';
import { TestDevicePlatform } from '../specs/e2e/multi-device/test-device-platform.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
// The local name that .github/actions/container-stage gives the same stage. A
// plain name, so that docker and podman both find it.
const IMAGE = process.env.FINI_ANDROID_PHONES_IMAGE ?? 'fini-android-e2e';
// By default this runner starts only virtual Android test devices: the job
// builds no Linux app, so a check of virtual Linux skips here.
process.env[E2eEnv.VirtualPlatforms] ??= TestDevicePlatform.Android;
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

const installed = (name: string) => spawnSync(name, ['--version'], { stdio: 'ignore' }).status === 0;

// The container engine: CONTAINER_ENGINE (podman|docker) when it is set,
// else podman when it is installed, else docker. CI uses podman.
// CONTAINER_ENGINE that is empty, `missing` (the value that the Makefile gives
// when it finds no engine) or not installed stops the script, as
// containers/run.sh does.
const ENGINE = process.env.CONTAINER_ENGINE ?? ['podman', 'docker'].find(installed) ?? '';
if (ENGINE === '' || ENGINE === 'missing' || !installed(ENGINE)) {
  console.error(`No container engine ('${ENGINE}'). Install podman or docker, or set CONTAINER_ENGINE=podman|docker.`);
  process.exit(1);
}
// Docker, or a docker command (`docker*`, as containers/run.sh finds it). All other engines are podman.
const isDocker = basename(ENGINE).startsWith('docker');
const isPodman = !isDocker;

// With podman: the flags of .github/actions/container-stage. FINI_CONTAINER_CACHE
// gives the GHCR layer cache, for example `ghcr.io/vruzhentsov/fini-cache`;
// without it the local layer cache only. FINI_CONTAINER_CACHE_PUSH=1 writes to
// it too. With docker: a plain `docker build` with the local cache of docker.
const cache = process.env.FINI_CONTAINER_CACHE ? `${process.env.FINI_CONTAINER_CACHE}/android` : '';
const podmanBuildArgs = isPodman
  ? [
    '--layers',
    ...(cache ? ['--cache-from', cache] : []),
    ...(cache && process.env.FINI_CONTAINER_CACHE_PUSH === '1' ? ['--cache-to', cache] : []),
    '--cache-ttl=168h', '--retry', '5', '--retry-delay', '5s',
  ]
  : [];
const built = run(ENGINE, [
  'build', ...podmanBuildArgs,
  '-f', 'containers/android/Containerfile', '--target', 'e2e', '-t', IMAGE, '.',
]);
if (built !== 0) process.exit(built);

// With docker, the run has the uid and gid of the user, so that the files in
// the repo do not belong to root, and $HOME is a named volume below /cache
// that gets this owner first, as in containers/run.sh. With rootless podman,
// root in the container is the user on the host.
const HOME_VOLUME = 'fini-home:/cache/home';
let userArgs: string[] = [];
if (isDocker) {
  const owner = `${process.getuid?.()}:${process.getgid?.()}`;
  const chowned = run(ENGINE, ['run', '--rm', '--user', '0:0', '-v', HOME_VOLUME, IMAGE, 'chown', owner, '/cache/home']);
  if (chowned !== 0) process.exit(chowned);
  userArgs = ['--user', owner, '-v', HOME_VOLUME, '-e', 'HOME=/cache/home'];
}

const envArgs = PASSED_ENV.filter((name) => process.env[name] !== undefined).flatMap((name) => ['-e', name]);
process.exit(run(ENGINE, [
  // --shm-size: with the default 64 MB of /dev/shm the emulator crashes
  // (SIGSEGV) at start.
  'run', '--rm', '--device', '/dev/kvm', '--shm-size=2g', ...userArgs,
  // With podman, `:z` gives the mount a shared SELinux label, as containers/run.sh does.
  '-v', `${REPO_ROOT}:/work${isPodman ? ':z' : ''}`, '-w', '/work', ...envArgs,
  IMAGE, 'node', 'scripts/e2e-android.mts', ...args,
]));
