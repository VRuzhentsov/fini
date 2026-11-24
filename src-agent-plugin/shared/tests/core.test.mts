import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import type { TestContext } from 'node:test';
import { execute, normalizeRequest, parseState, STATE_ERROR } from '../core.mts';
import { writeState } from '../store.mts';

const CLI = fileURLToPath(new URL('../cli.mts', import.meta.url));
const UUID = '60eba7ee-1dd7-48fb-969d-e68a263ed0a0';
function fixture(t: TestContext) {
  assert.ok(process.env.TMPDIR, 'Set TMPDIR to the workstation scratch directory.');
  const root = mkdtempSync(join(process.env.TMPDIR, 'fini-ts-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return { root, state_path: join(root, 'state', 'sessions.json') };
}
function cli(request: unknown): Record<string, unknown> {
  const result = spawnSync(process.execPath, [CLI], {
    input: JSON.stringify(request), encoding: 'utf8', timeout: 15_000,
  });
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}
function call(request: unknown): Record<string, unknown> {
  const result = execute(request) as Record<string, unknown>;
  assert.ok(!('error' in result), JSON.stringify(result));
  return result;
}

test('direct CLI preserves literal IDs, multiple providers, per-provider dedup, legacy listing and modes', t => {
  const { state_path } = fixture(t);
  for (const quest_id of [' 0001 / 漢字 "quoted" ', '__proto__', 'constructor', 'toString']) {
    for (const provider of ['hermes', 'claude']) {
      for (const session_id of [UUID, UUID.toUpperCase()]) {
        const request = { state_path, action: 'link', quest_id, provider, session_id };
        assert.equal(cli(request).added, true);
        const before = readFileSync(state_path);
        assert.equal(cli(request).added, false);
        assert.deepEqual(readFileSync(state_path), before);
      }
    }
    const result = cli({ state_path, action: 'list', quest_id });
    assert.deepEqual(result.session_ids, [UUID, UUID.toUpperCase()]);
    assert.equal((result.sessions as unknown[]).length, 4);
  }
  assert.equal(statSync(state_path).mode & 0o777, 0o600);
});

test('core validates direct and slash inputs before creating storage', t => {
  const { root, state_path } = fixture(t);
  const bad = [null, [], {}, { state_path: 'relative', action: 'list', quest_id: 'q' },
    { state_path, action: 'link', quest_id: 'q', provider: 'other', session_id: 's' },
    { state_path, action: 'link', quest_id: 'q', provider: 'hermes', session_id: null },
    { state_path, action: 'link', quest_id: 'q', provider: 'claude', session_id: ` ${UUID}` },
    { state_path, action: 'list', quest_id: ' ' },
    { state_path, action: 'list', quest_id: 'q', extra: true },
    { state_path, command: 'fini-link', raw_args: 'q', session_id: null },
    ...['[]', '{}', '{broken', JSON.stringify({ quest_id: 'q', session_id: UUID, extra: 1 })].map(raw_args =>
      ({ state_path, command: 'fini-link-claude', raw_args, session_id: null }))];
  for (const value of bad) assert.equal(typeof cli(value).error, 'string', JSON.stringify(value));
  assert.deepEqual(readdirSync(root), []);
  assert.throws(() => normalizeRequest({ state_path, action: 'list', quest_id: 1 }));
});

test('Python-compatible whitespace validation and exact UUID end matching', t => {
  const { state_path } = fixture(t);
  for (const quest_id of ['\u0085', '\u001c\u001f']) {
    assert.equal(typeof cli({ state_path, action: 'list', quest_id }).error, 'string');
  }
  assert.equal(cli({ state_path, action: 'link', quest_id: '\ufeff', provider: 'hermes', session_id: '\ufeff' }).added, true);
  for (const suffix of ['\n', '\r', '\u2028', ' ']) {
    assert.equal(typeof cli({ state_path, action: 'link', quest_id: 'q', provider: 'claude', session_id: UUID + suffix }).error, 'string');
  }
});

test('v1 listing never migrates; duplicate links migrate preserving all quests', t => {
  const { root, state_path } = fixture(t);
  mkdirSync(join(root, 'state'));
  const original = '{"version":1,"quests":{"q":["old"],"other":["preserved"]}}';
  writeFileSync(state_path, original);
  assert.deepEqual(cli({ state_path, action: 'list', quest_id: 'q' }).session_ids, ['old']);
  assert.equal(readFileSync(state_path, 'utf8'), original);
  assert.equal(cli({ state_path, action: 'link', quest_id: 'q', provider: 'hermes', session_id: 'old' }).added, false);
  assert.deepEqual(JSON.parse(readFileSync(state_path, 'utf8')), {
    version: 2, quests: { q: { hermes: ['old'] }, other: { hermes: ['preserved'] } },
  });
});

test('malformed schemas and invalid UTF-8 remain byte-for-byte intact', t => {
  const { root, state_path } = fixture(t);
  mkdirSync(join(root, 'state'));
  const corrupt = ['{broken', '[]', '{"version":true,"quests":{}}', '{"version":3,"quests":{}}',
    '{"version":2,"quests":{},"extra":1}', '{"version":2,"quests":{"q":{"unknown":[]}}}',
    '{"version":2,"quests":{"q":{"claude":["bad"]}}}', '{"version":1,"quests":{"q":[2]}}',
    '{"version":2,"quests":{" ":{}}}', '{"version":2,"quests":{"q":[]}}'];
  for (const text of corrupt) assert.throws(() => parseState(text), { message: STATE_ERROR });
  for (const bytes of [...corrupt.map(text => Buffer.from(text)), Buffer.from([0xff, 0xfe])]) {
    writeFileSync(state_path, bytes);
    for (const action of ['link', 'list']) {
      const request = action === 'list' ? { action, quest_id: 'q', state_path }
        : { action, quest_id: 'q', state_path, provider: 'hermes', session_id: 's' };
      assert.equal(typeof cli(request).error, 'string');
      assert.deepEqual(readFileSync(state_path), bytes);
    }
  }
});

test('replacement failure preserves original file and cleans private temporary output', t => {
  const { root, state_path } = fixture(t);
  call({ state_path, action: 'link', quest_id: 'q', provider: 'hermes', session_id: 'original' });
  const before = readFileSync(state_path);
  assert.throws(() => writeState(state_path, parseState(before.toString()), () => { throw new Error('injected rename failure'); }),
    /injected rename failure/);
  assert.deepEqual(readFileSync(state_path), before);
  assert.deepEqual(readdirSync(join(root, 'state')).filter(name => name.startsWith('.sessions-')), []);
});

test('independent CLI processes serialize mixed-provider migration with no lost updates', async t => {
  const { root, state_path } = fixture(t);
  mkdirSync(join(root, 'state'));
  writeFileSync(state_path, '{"version":1,"quests":{"q":["original"]}}');
  const requests = Array.from({ length: 48 }, (_, index) => ({
    state_path, action: 'link', quest_id: 'q', provider: index % 2 ? 'claude' : 'hermes',
    session_id: `00000000-0000-0000-0000-${(index % 24).toString(16).padStart(12, '0')}`,
  }));
  await Promise.all(requests.map(async request => {
    const worker = spawn(process.execPath, [CLI], { stdio: ['pipe', 'pipe', 'pipe'] });
    const closed = once(worker, 'close');
    let out = '', err = '';
    worker.stdout.setEncoding('utf8').on('data', chunk => { out += chunk; });
    worker.stderr.setEncoding('utf8').on('data', chunk => { err += chunk; });
    worker.stdin.end(JSON.stringify(request));
    const [code] = await closed;
    assert.equal(code, 0, err);
    assert.ok(!JSON.parse(out).error, out);
  }));
  const state = parseState(readFileSync(state_path, 'utf8'));
  assert.equal(state.version, 2);
  assert.equal(state.quests.q?.hermes?.length, 13);
  assert.equal(state.quests.q?.claude?.length, 12);
  assert.ok(state.quests.q?.hermes?.includes('original'));
});

test('OS-backed lock never breaks a live owner; SIGKILL releases same sidecar', async t => {
  const { root, state_path } = fixture(t);
  mkdirSync(join(root, 'state'));
  const lock = join(root, 'state', 'sessions.lock');
  writeFileSync(lock, '');
  const owner = spawn('flock', ['--no-fork', lock, process.execPath, '-e',
    'process.stdout.write("locked\\n");setInterval(()=>{},1000)'], { stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => { if (owner.exitCode === null) owner.kill('SIGKILL'); });
  await once(owner.stdout, 'data');
  const inode = statSync(lock).ino;
  const result = cli({ state_path, action: 'link', quest_id: 'q', provider: 'hermes', session_id: 's' });
  assert.match(String(result.error), /busy/);
  assert.equal(statSync(lock).ino, inode);
  assert.deepEqual(readdirSync(join(root, 'state')), ['sessions.lock']);
  const closed = once(owner, 'close'); owner.kill('SIGKILL'); await closed;
  assert.equal(cli({ state_path, action: 'link', quest_id: 'q', provider: 'hermes', session_id: 's' }).added, true);
  assert.equal(statSync(lock).ino, inode);
});

test('malformed JSON and oversized CLI requests return bounded JSON errors', t => {
  fixture(t);
  for (const input of ['{bad', ' '.repeat(1024 * 1024 + 1)]) {
    const result = spawnSync(process.execPath, [CLI], { input, encoding: 'utf8', timeout: 15_000 });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(typeof JSON.parse(result.stdout).error, 'string');
    assert.ok(result.stdout.length < 1024);
  }
});
