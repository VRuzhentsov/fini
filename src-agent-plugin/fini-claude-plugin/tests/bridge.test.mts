import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
const root = dirname(dirname(dirname(fileURLToPath(import.meta.url))));
const source = join(root, 'fini-claude-plugin', 'bridge.mts');
const sid = '12345678-1234-1234-1234-123456789aBc';
const hook = (extra = {}) => ({ hook_event_name: 'UserPromptExpansion',
  expansion_type: 'slash_command', command_source: 'plugin', command_name: 'fini:link',
  command_args: '001', session_id: sid, ...extra });
function environment(home: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: home, HERMES_HOME: join(home, '.hermes') };
  for (const key of ['FINI_SESSIONS_PATH', 'HERMES_PROFILE', 'HERMES_DATA_DIR_SUFFIX']) delete env[key];
  return env;
}
function run(input: unknown, env: NodeJS.ProcessEnv, script = source) {
  const p = spawnSync(process.execPath, [script], { input: typeof input === 'string' || Buffer.isBuffer(input) ? input : JSON.stringify(input),
    encoding: 'utf8', env, timeout: 22_000, maxBuffer: 3 * 1024 * 1024 });
  assert.equal(p.error, undefined); assert.equal(p.status, 0, p.stderr);
  const output = JSON.parse(p.stdout); assert.equal(output.decision, 'block');
  return JSON.parse(output.reason);
}
function temporary(t: { after(fn: () => void): void }) {
  const home = mkdtempSync(join(tmpdir(), 'fini-claude-'));
  t.after(() => rmSync(home, { recursive: true, force: true }));
  return home;
}
test('native hook uses runtime UUID only, literal quest, multiple sessions and provider-qualified listing', t => {
  const home = temporary(t), env = environment(home);
  const quest = '  001 / 漢字 "quoted"; $(touch nope)  ';
  const first = run(hook({ command_args: quest, prompt: 'fake UUID', transcript_path: '/not/read' }), env);
  assert.equal(first.quest_id, quest); assert.equal(first.session_id, sid); assert.equal(first.added, true);
  const before = readFileSync(first.state_path);
  assert.equal(run(hook({ command_args: quest }), env).added, false);
  assert.deepEqual(readFileSync(first.state_path), before);
  const second = randomUUID();
  assert.equal(run(hook({ command_args: quest, session_id: second }), env).added, true);
  const list = run(hook({ command_name: 'fini:sessions', command_args: quest, session_id: undefined }), env);
  assert.deepEqual(list.sessions, [{ provider: 'claude', session_id: sid }, { provider: 'claude', session_id: second }]);
  assert.equal(first.state_path, join(home, '.hermes', 'plugin-data', 'fini-hermes-plugin', 'sessions.json'));
});
test('missing or invalid identity/context and blank quest fail before storage; no env/transcript fallback', t => {
  const home = temporary(t), env = environment(home);
  env.CLAUDE_SESSION_ID = sid;
  for (const extra of [{ session_id: undefined }, { session_id: 'bad' }, { session_id: sid + '\n' },
    { session_id: null }, { command_args: '' }, { command_args: '   ' }, { command_args: 1 },
    { hook_event_name: 'UserPromptSubmit' }, { expansion_type: 'mcp_prompt' },
    { command_source: 'project' }, { command_name: 'fini:other' }]) {
    assert.ok(run(hook(extra), env).error, JSON.stringify(extra));
  }
  assert.equal(existsSync(env.HERMES_HOME!), false);
});
test('default home, explicit common file and profile boundaries; invalid overrides never fallback', t => {
  const home = temporary(t), env = environment(home);
  delete env.HERMES_HOME;
  assert.equal(run(hook(), env).state_path, join(home, '.hermes', 'plugin-data', 'fini-hermes-plugin', 'sessions.json'));
  const isolated = join(home, 'profile-b');
  env.HERMES_HOME = isolated;
  assert.deepEqual(run(hook({ command_name: 'fini:sessions' }), env).sessions, []);
  env.HERMES_PROFILE = 'automation';
  assert.ok(run(hook(), env).error);
  assert.equal(existsSync(join(isolated, 'plugin-data')), true); // listing creates only lock directory
  env.FINI_SESSIONS_PATH = join(home, 'explicit', 'sessions.json');
  assert.equal(run(hook(), env).state_path, env.FINI_SESSIONS_PATH);
  for (const path of ['', 'relative.json', '~/sessions.json']) {
    env.FINI_SESSIONS_PATH = path; assert.ok(run(hook(), env).error);
  }
});
test('malformed input, invalid UTF-8 and input size limit return blocking JSON', t => {
  const home = temporary(t), env = environment(home);
  for (const raw of ['{bad', '[]', Buffer.from([0xff]), 'x'.repeat(1024 * 1024 + 1)]) assert.ok(run(raw, env).error);
  assert.equal(existsSync(env.HERMES_HOME!), false);
});
test('ambiguous suffix/profile and invalid home fail closed without profile discovery', t => {
  const home = temporary(t);
  for (const extra of [{ HERMES_DATA_DIR_SUFFIX: '-other' }, { HERMES_PROFILE: 'other' },
    { HERMES_HOME: '' }, { HERMES_HOME: '~/custom' }]) {
    assert.ok(run(hook(), { ...environment(home), ...extra }).error);
  }
  assert.equal(existsSync(join(home, '.hermes')), false);
});
test('native bridge propagates the real core lock deadline without writing', { timeout: 15_000 }, async t => {
  const home = temporary(t), env = environment(home);
  const directory = join(home, 'common'); mkdirSync(directory);
  env.FINI_SESSIONS_PATH = join(directory, 'sessions.json');
  const owner = spawn('flock', ['--exclusive', '--no-fork', join(directory, 'sessions.lock'),
    process.execPath, '-e', 'console.log("locked"); setInterval(() => {}, 1000)']);
  try {
    await new Promise<void>((resolve, reject) => {
      owner.stdout.once('data', () => resolve()); owner.once('error', reject);
      owner.once('exit', () => reject(new Error('Lock owner exited before bridge test.')));
    });
    const started = Date.now();
    assert.match(run(hook(), env).error, /busy/);
    assert.ok(Date.now() - started < 12_000);
    assert.equal(existsSync(env.FINI_SESSIONS_PATH), false);
  } finally {
    owner.kill('SIGKILL');
    await new Promise(resolve => owner.once('exit', resolve));
  }
});
test('escaped hook output is bounded even after a committed large literal quest link', t => {
  const home = temporary(t), env = environment(home);
  const quest = '"'.repeat(300_000);
  const result = run(hook({ command_args: quest }), env);
  assert.match(result.error, /exceeds 1 MiB/);
  const path = join(env.HERMES_HOME!, 'plugin-data', 'fini-hermes-plugin', 'sessions.json');
  // Error is a transport boundary, not a rollback claim.
  assert.deepEqual(JSON.parse(readFileSync(path, 'utf8')).quests[quest].claude, [sid]);
});
test('EOF deadline bounds stalled hook stdin without writing', { timeout: 20_000 }, async t => {
  const home = temporary(t), env = environment(home);
  const p = spawn(process.execPath, [source], { env, stdio: ['pipe', 'pipe', 'pipe'] });
  let text = ''; p.stdout.on('data', chunk => { text += chunk; });
  p.stdin.write('{');
  const status = await new Promise<number | null>(resolve => p.on('exit', resolve));
  p.stdin.destroy();
  assert.equal(status, 0); assert.match(JSON.parse(JSON.parse(text).reason).error, /timed out/);
  assert.equal(existsSync(env.HERMES_HOME!), false);
});
test('assembled cache root includes byte-identical canonical code and survives relocation', t => {
  const home = temporary(t);
  const build = spawnSync(process.execPath, [join(root, 'fini-claude-plugin', 'package.mts')], { encoding: 'utf8' });
  assert.equal(build.status, 0, build.stderr);
  const packaged = build.stdout.trim(), cache = join(home, 'cache with spaces', 'fini', '0.4.0');
  cpSync(packaged, cache, { recursive: true });
  const hashes = JSON.parse(readFileSync(join(cache, 'source-hashes.json'), 'utf8')) as Record<string, string>;
  for (const [name, hash] of Object.entries(hashes)) {
    assert.deepEqual(readFileSync(join(cache, name)), readFileSync(join(root, name)));
    assert.equal(createHash('sha256').update(readFileSync(join(cache, name))).digest('hex'), hash);
  }
  const env = environment(home);
  const result = run(hook(), env, join(cache, 'fini-claude-plugin', 'bridge.mts'));
  assert.equal(result.session_id, sid); assert.equal(result.added, true);
  const config = JSON.parse(readFileSync(join(cache, 'hooks', 'hooks.json'), 'utf8'));
  const command = config.hooks.UserPromptExpansion[0].hooks[0].command.replace('${CLAUDE_PLUGIN_ROOT}', cache);
  const native = spawnSync('/bin/sh', ['-c', command], { env, input: JSON.stringify(hook()), encoding: 'utf8' });
  assert.equal(native.status, 0, native.stderr);
  assert.equal(JSON.parse(JSON.parse(native.stdout).reason).added, false);
});
