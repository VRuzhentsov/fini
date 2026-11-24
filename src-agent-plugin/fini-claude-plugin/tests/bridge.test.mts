import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
const root = fileURLToPath(new URL('../../', import.meta.url));
const source = join(root, 'fini-claude-plugin', 'bridge.mts');
const sid = '12345678-1234-1234-1234-123456789aBc';
const hook = (extra = {}) => ({ hook_event_name: 'UserPromptExpansion',
  expansion_type: 'slash_command', command_source: 'plugin', command_name: 'fini:link',
  command_args: '', session_id: sid, ...extra });
function fixture(t: { after(fn: () => void): void }) {
  assert.ok(process.env.TMPDIR);
  const home = mkdtempSync(join(process.env.TMPDIR, 'fini-claude-'));
  t.after(() => rmSync(home, { recursive: true, force: true }));
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: home, HERMES_HOME: join(home, 'profile-a'),
    XDG_DATA_HOME: join(home, 'data'), XDG_CONFIG_HOME: join(home, 'config') };
  delete env.FINI_AGENT_SESSIONS_DB;
  const p = spawnSync('fini', ['quest', 'create', '--title', 'Claude check', '--json'], { env, encoding: 'utf8' });
  assert.equal(p.status, 0, p.stderr);
  return { home, env, quest: JSON.parse(p.stdout).id as string };
}
function run(input: unknown, env: NodeJS.ProcessEnv, script = source) {
  const p = spawnSync(process.execPath, [script], { input: typeof input === 'string' || Buffer.isBuffer(input) ? input : JSON.stringify(input),
    encoding: 'utf8', env, timeout: 22_000, maxBuffer: 3 * 1024 * 1024 });
  assert.equal(p.status, 0, p.stderr);
  const output = JSON.parse(p.stdout); assert.equal(output.decision, 'block');
  return JSON.parse(output.reason);
}

test('hook context, all commands, runtime identity and shared cross-profile default', t => {
  const { home, env, quest } = fixture(t);
  const first = run(hook({ command_args: quest, transcript_path: '/not/read', prompt: 'wrong' }), env);
  assert.equal(first.session_id, sid); assert.equal(first.changed, true);
  assert.equal(first.db_path, join(home, '.fini', 'agent-sessions.db'));
  env.HERMES_HOME = join(home, 'profile-b'); env.HERMES_PROFILE = 'different';
  assert.equal(run(hook({ command_args: quest }), env).changed, false);
  const explicit = `${quest} --agent hermes --session ' exact 001 '`;
  assert.equal(run(hook({ command_args: explicit, session_id: undefined }), env).session_id, ' exact 001 ');
  assert.equal(run(hook({ command_name: 'fini:relations', command_args: '' }), env).quests.length, 1);
  assert.equal(run(hook({ command_name: 'fini:primary', command_args: `${quest} --clear` }), env).agents[0].primary, null);
  assert.equal(run(hook({ command_name: 'fini:unlink', command_args: quest }), env).changed, true);
  assert.equal(run(hook({ command_name: 'fini:relations', command_args: `--quest ${quest}`, session_id: undefined }), env).agents.length, 1);
});

test('invalid native context and bounded input fail before database writes', t => {
  const { home, env, quest } = fixture(t);
  env.CLAUDE_SESSION_ID = sid;
  for (const extra of [{ session_id: undefined }, { session_id: 'bad' }, { session_id: sid + '\n' },
    { command_args: `${quest} extra` }, { hook_event_name: 'UserPromptSubmit' },
    { expansion_type: 'mcp_prompt' }, { command_source: 'project' }, { command_name: 'fini:sessions' }]) {
    assert.ok(run(hook({ command_args: quest, ...extra }), env).error);
  }
  for (const input of ['{bad', '[]', Buffer.from([0xff]), 'x'.repeat(1024 * 1024 + 1)]) assert.ok(run(input, env).error);
  assert.equal(existsSync(join(home, '.fini')), false);
});

test('assembled native commands and production dependencies work after relocation', t => {
  const { home, env, quest } = fixture(t);
  const p = spawnSync(process.execPath, [join(root, 'fini-claude-plugin', 'package.mts')], { env, encoding: 'utf8' });
  assert.equal(p.status, 0, p.stderr);
  const cache = join(home, 'cache with spaces', 'fini'); cpSync(p.stdout.trim(), cache, { recursive: true });
  const hashes = JSON.parse(readFileSync(join(cache, 'source-hashes.json'), 'utf8')) as Record<string, string>;
  for (const [name, hash] of Object.entries(hashes)) {
    assert.deepEqual(readFileSync(join(cache, name)), readFileSync(join(root, name)));
    assert.equal(createHash('sha256').update(readFileSync(join(cache, name))).digest('hex'), hash);
  }
  for (const command of ['link', 'unlink', 'primary', 'relations']) assert.ok(existsSync(join(cache, 'commands', `${command}.md`)));
  assert.equal(existsSync(join(cache, 'skills')), false);
  assert.ok(existsSync(join(cache, 'node_modules', 'drizzle-orm')));
  assert.equal(run(hook({ command_args: quest }), env, join(cache, 'fini-claude-plugin', 'bridge.mts')).changed, true);
});
