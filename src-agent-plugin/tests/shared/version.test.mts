import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const read = (path: string) => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');

test('Claude, Hermes and the development package have the same plugin version', () => {
  const claude = (JSON.parse(read('fini/.claude-plugin/plugin.json')) as { version: string }).version;
  const hermes = /^version:\s*(\S+)\s*$/m.exec(read('fini/plugin.yaml'))?.[1];
  const npm = (JSON.parse(read('package.json')) as { version: string }).version;
  assert.equal(hermes, claude, 'fini/plugin.yaml');
  assert.equal(npm, claude, 'package.json');
});
