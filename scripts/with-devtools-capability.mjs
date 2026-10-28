// Runs one command with the devtools Tauri capability in place of the
// committed one, and puts the committed one back however the command ends.
// Same job as `with_devtools_capability` in the Makefile, for npm scripts.
//
//   node scripts/with-devtools-capability.mjs <command> [args...]

import { spawn } from 'node:child_process';
import { copyFileSync, readFileSync, writeFileSync } from 'node:fs';

const CAPABILITY = 'src-tauri/capabilities/default.json';
const DEVTOOLS_CAPABILITY = 'src-tauri/devtools-capabilities/default.json';

const [command, ...args] = process.argv.slice(2);
if (!command) {
  console.error('usage: with-devtools-capability.mjs <command> [args...]');
  process.exit(2);
}

const committed = readFileSync(CAPABILITY);
let restored = false;
const restore = () => {
  if (restored) return;
  restored = true;
  writeFileSync(CAPABILITY, committed);
};

copyFileSync(DEVTOOLS_CAPABILITY, CAPABILITY);
const child = spawn(command, args, { stdio: 'inherit' });

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => child.kill(signal));
}
child.on('error', (error) => {
  restore();
  console.error(error.message);
  process.exit(1);
});
child.on('exit', (code, signal) => {
  restore();
  process.exit(code ?? (signal ? 1 : 0));
});
