import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
const root = fileURLToPath(new URL('../../', import.meta.url));

test('assembled native commands and production dependencies work after relocation', t => {
  assert.ok(process.env.TMPDIR);
  const home = mkdtempSync(join(process.env.TMPDIR, 'fini-claude-'));
  t.after(() => rmSync(home, { recursive: true, force: true }));
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: home, HERMES_HOME: join(home, 'hermes'),
    XDG_DATA_HOME: join(home, 'data'), XDG_CONFIG_HOME: join(home, 'config') };
  delete env.FINI_AGENT_SESSIONS_DB;
  const created = spawnSync('fini', ['quest', 'create', '--title', 'Claude check', '--json'], { env, encoding: 'utf8' });
  assert.equal(created.status, 0, created.stderr);
  const quest = JSON.parse(created.stdout).id as string;
  const p = spawnSync(process.execPath, [join(root, 'fini-claude-plugin', 'package.mts')], { env, encoding: 'utf8' });
  assert.equal(p.status, 0, p.stderr);
  const cache = join(home, 'cache with spaces', 'fini'); cpSync(p.stdout.trim(), cache, { recursive: true });
  const hashes = JSON.parse(readFileSync(join(cache, 'source-hashes.json'), 'utf8')) as Record<string, string>;
  for (const [name, hash] of Object.entries(hashes)) {
    assert.deepEqual(readFileSync(join(cache, name)), readFileSync(join(root, name)));
    assert.equal(createHash('sha256').update(readFileSync(join(cache, name))).digest('hex'), hash);
  }
  for (const command of ['link', 'unlink', 'primary', 'relations']) assert.ok(existsSync(join(cache, 'commands', `${command}.md`)));
  assert.ok(existsSync(join(cache, 'hooks', 'register.js')));
  assert.equal(existsSync(join(cache, 'skills')), false);
  assert.ok(existsSync(join(cache, 'node_modules', 'drizzle-orm')));
  // The mod calls exactly this transport with the runtime session ID.
  const request = { command: 'fini:link', raw_args: quest, current: { agent: 'claude', session_id: '12345678-1234-1234-1234-123456789abc' } };
  const linked = spawnSync(process.execPath, [join(cache, 'shared', 'cli.mts')], { env, input: JSON.stringify(request), encoding: 'utf8' });
  assert.equal(linked.status, 0, linked.stderr);
  assert.equal(JSON.parse(linked.stdout).changed, true);
});
