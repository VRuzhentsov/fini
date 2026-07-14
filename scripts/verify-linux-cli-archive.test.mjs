import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const script = new URL('./verify-linux-cli-archive.sh', import.meta.url).pathname;

async function withArchive(contents, fn) {
  const dir = await mkdtemp(join(tmpdir(), 'fini-cli-archive-'));
  try {
    const binary = join(dir, 'fini');
    const archive = join(dir, 'fini-cli.tar.gz');
    await writeFile(binary, contents, 'utf8');
    await chmod(binary, 0o755);
    const packed = spawnSync('tar', ['-czf', archive, '-C', dir, 'fini'], {
      encoding: 'utf8',
    });
    assert.equal(packed.status, 0, packed.stderr);
    return await fn(archive);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test('accepts a staged Linux CLI archive only when fini --version matches', async () => {
  await withArchive('#!/usr/bin/env sh\nprintf "fini 1.2.3\\n"\n', async (archive) => {
    const result = spawnSync('bash', [script, archive, '1.2.3'], { encoding: 'utf8' });

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /fini --version exit=0/);
    assert.match(result.stdout, /stdout=fini 1\.2\.3/);
  });
});
