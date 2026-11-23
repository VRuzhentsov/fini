// Builds the app for the e2e tests, with the devtools capability
// (scripts/with-devtools-capability.mts). One target for each npm script:
//
//   node scripts/e2e-build.mts desktop   npm run build:desktop:e2e
//   node scripts/e2e-build.mts windows   npm run build:windows:e2e
//   node scripts/e2e-build.mts android   npm run build:android:e2e
//
// The arguments after the target go to the last command of the target.
//
// `desktop` puts the binaries of the app, the CLI and the mock broker in
// src-tauri/target/debug-e2e/debug, where the e2e providers find them.

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const WITH_DEVTOOLS = ['scripts/with-devtools-capability.mts', 'npm', 'run', 'tauri'];

function run(command: string, args: string[], env: Record<string, string | undefined> = process.env): void {
  const result = spawnSync(command, args, { stdio: 'inherit', cwd: REPO_ROOT, env });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

const withDevtools = (args: string[]) => run(process.execPath, [...WITH_DEVTOOLS, ...args]);

const targets: Record<string, (args: string[]) => void> = {
  desktop: (args) => {
    const env = { ...process.env, CARGO_TARGET_DIR: join(REPO_ROOT, 'src-tauri', 'target', 'debug-e2e') };
    run(process.execPath, [
      ...WITH_DEVTOOLS, '--',
      'build', '--debug', '--features', 'ui-plane,desktop-updater,devtools', '--no-bundle', '--', '--bin', 'fini-app',
    ], env);
    run('cargo', ['build', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', 'fini', '--features', 'cli-plane'], env);
    run('cargo', ['build', '--manifest-path', 'ble-mock-broker/Cargo.toml', ...args], env);
  },

  windows: (args) => withDevtools([
    'build', '--', '--debug', '--features', 'ui-plane,devtools', '--no-bundle', '--', '--bin', 'fini-app', ...args,
  ]),

  android: (args) => withDevtools([
    'android', 'build', '--', '--features', 'ui-plane,devtools', '--ci', '--debug', '--apk', '--target', 'x86_64', ...args,
  ]),
};

const [target, ...args] = process.argv.slice(2);
const build = targets[target];
if (!build) {
  console.error(`usage: e2e-build.mts <${Object.keys(targets).join('|')}> [args...]`);
  process.exit(2);
}
build(args);
