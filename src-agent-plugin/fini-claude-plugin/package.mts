#!/usr/bin/env node
// Assemble a self-contained cacheable plugin from the sole reviewed sources.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const output = join(root, '.artifacts', 'claude', 'fini');
rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });
for (const name of ['.claude-plugin', 'hooks', 'commands', 'README.md']) {
  cpSync(join(root, 'fini-claude-plugin', name), join(output, name), { recursive: true });
}
const files = ['shared/core.mts', 'shared/store.mts', 'shared/transport.mts', 'shared/cli.mts'];
const hashes: Record<string, string> = {};
for (const name of files) {
  mkdirSync(dirname(join(output, name)), { recursive: true });
  const bytes = readFileSync(join(root, name));
  writeFileSync(join(output, name), bytes);
  hashes[name] = createHash('sha256').update(bytes).digest('hex');
}
// Include the locked production packages, including the SQLite native binaries.
// No source-checkout path or network install is needed after cache relocation.
const lock = JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8')) as {
  packages: Record<string, { dev?: boolean; devOptional?: boolean }>;
};
for (const [name, entry] of Object.entries(lock.packages)) {
  if (!name || entry.dev || entry.devOptional) continue;
  cpSync(join(root, name), join(output, name), { recursive: true, dereference: true });
}
writeFileSync(join(output, 'package.json'), JSON.stringify({ private: true, type: 'module' }));
writeFileSync(join(output, 'source-hashes.json'), `${JSON.stringify(hashes, null, 2)}\n`);
// An optional local marketplace, pointing ONLY at the assembled root.
mkdirSync(join(root, '.artifacts', 'claude', '.claude-plugin'), { recursive: true });
const manifest = JSON.parse(readFileSync(join(output, '.claude-plugin', 'plugin.json'), 'utf8')) as { name: string; version: string };
writeFileSync(join(root, '.artifacts', 'claude', '.claude-plugin', 'marketplace.json'),
  `${JSON.stringify({ name: 'fini-local', owner: { name: 'Fini' },
    plugins: [{ name: manifest.name, source: './fini', version: manifest.version }] }, null, 2)}\n`);
console.log(output);
