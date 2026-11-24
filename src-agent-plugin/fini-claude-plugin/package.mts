#!/usr/bin/env node
// Assemble a self-contained cacheable plugin from the sole reviewed sources.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const output = join(root, '.artifacts', 'claude', 'fini');
rmSync(output, { recursive: true, force: true });
mkdirSync(join(output, 'fini-claude-plugin'), { recursive: true });
for (const name of ['.claude-plugin', 'hooks', 'skills', 'README.md']) {
  cpSync(join(root, 'fini-claude-plugin', name), join(output, name), { recursive: true });
}
const files = ['fini-claude-plugin/bridge.mts', 'shared/core.mts', 'shared/store.mts',
  'shared/locked-worker.mts', 'shared/transport.mts', 'shared/cli.mts'];
const hashes: Record<string, string> = {};
for (const name of files) {
  mkdirSync(dirname(join(output, name)), { recursive: true });
  const bytes = readFileSync(join(root, name));
  writeFileSync(join(output, name), bytes);
  hashes[name] = createHash('sha256').update(bytes).digest('hex');
}
writeFileSync(join(output, 'source-hashes.json'), `${JSON.stringify(hashes, null, 2)}\n`);
// An optional local marketplace, pointing ONLY at the assembled root.
mkdirSync(join(root, '.artifacts', 'claude', '.claude-plugin'), { recursive: true });
const manifest = JSON.parse(readFileSync(join(output, '.claude-plugin', 'plugin.json'), 'utf8')) as { name: string; version: string };
writeFileSync(join(root, '.artifacts', 'claude', '.claude-plugin', 'marketplace.json'),
  `${JSON.stringify({ name: 'fini-local', owner: { name: 'Fini' },
    plugins: [{ name: manifest.name, source: './fini', version: manifest.version }] }, null, 2)}\n`);
console.log(output);
