import assert from 'node:assert/strict';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const verifier = new URL('./verify-cli-cargo-dependency-boundary.sh', import.meta.url);

async function withDependencyTree(content, fn) {
  const dir = await mkdtemp(join(tmpdir(), 'fini-cli-dependency-boundary-'));
  const tree = join(dir, 'tree.txt');
  try {
    await writeFile(tree, content, 'utf8');
    return await fn(tree);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test('rejects desktop runtime dependencies from the CLI dependency graph', async () => {
  await withDependencyTree('fini\n├── tauri-plugin-updater\n└── webkit2gtk\n', async (tree) => {
    const result = spawnSync(verifier.pathname, ['--tree-file', tree], { encoding: 'utf8' });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /tauri-plugin-updater/);
    assert.match(result.stderr, /webkit2gtk/);
  });
});

test('accepts a headless CLI dependency graph', async () => {
  await withDependencyTree('fini\n├── self_update\n└── self-replace\n', async (tree) => {
    const result = spawnSync(verifier.pathname, ['--tree-file', tree], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
  });
});
