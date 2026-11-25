import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import type { TestContext } from 'node:test';
import Database from 'better-sqlite3';
import { Command, RelationCommand, execute } from '../core.mts';
import { SqliteRelationsRepository } from '../store.mts';

const CLI = fileURLToPath(new URL('../cli.mts', import.meta.url));
const uuid = '60eba7ee-1dd7-48fb-969d-e68a263ed0a0';
function fixture(t: TestContext) {
  assert.ok(process.env.TMPDIR);
  const home = mkdtempSync(join(process.env.TMPDIR, 'fini-core-'));
  t.after(() => rmSync(home, { recursive: true, force: true }));
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: home, HERMES_HOME: join(home, 'hermes'),
    XDG_DATA_HOME: join(home, 'data'), XDG_CONFIG_HOME: join(home, 'config'),
    FINI_AGENT_SESSIONS_DB: join(home, '.fini', 'agent-sessions.db') };
  const quests = ['first', 'second'].map(title => {
    const p = spawnSync('fini', ['quest', 'create', '--title', title, '--json'], { env, encoding: 'utf8' });
    assert.equal(p.status, 0, p.stderr); return JSON.parse(p.stdout).id as string;
  });
  const current = { agent: 'hermes', session_id: 'runtime-a' };
  function call(command: string, raw_args: string, identity: unknown = current, extra = {}, environment = env) {
    const p = spawnSync(process.execPath, [CLI], { env: environment, input: JSON.stringify({
      command, raw_args, current: identity, ...extra,
    }), encoding: 'utf8', timeout: 20_000 });
    assert.equal(p.status, 0, p.stderr); return JSON.parse(p.stdout);
  }
  return { home, env, quests, call, path: env.FINI_AGENT_SESSIONS_DB! };
}

test('many-to-many relations and primary lifecycle through the real Fini CLI', t => {
  const { call, quests: [q, other], path } = fixture(t);
  assert.ok(q && other);
  assert.equal(call(Command.Link, q).changed, true);
  call(Command.Link, `${q} --agent hermes --session runtime-b`);
  call(Command.Link, `${q} --agent claude --session ${uuid}`);
  call(Command.Link, other);
  assert.equal(call(Command.Link, q).changed, false);
  assert.deepEqual(call(Command.Relations, `--quest ${q}`).agents, [
    { agent: 'claude', primary: uuid, related: [] },
    { agent: 'hermes', primary: 'runtime-a', related: ['runtime-b'] },
  ]);
  assert.equal(call(Command.Relations, '').quests.length, 2);
  assert.equal(call(Command.Relations, `--agent claude --session ${uuid}`).quests.length, 1);
  call(Command.Primary, `${q} --agent hermes --session runtime-b`);
  assert.equal(call(Command.Link, q).changed, false);
  assert.equal(call(Command.Relations, `--quest ${q}`).agents[1].primary, 'runtime-b');
  assert.ok(call(Command.Primary, `${q} --agent hermes --session absent`).error);
  assert.equal(call(Command.Relations, `--quest ${q}`).agents[1].primary, 'runtime-b');
  call(Command.Primary, `${q} --clear`);
  assert.deepEqual(call(Command.Relations, `--quest ${q}`).agents[1], {
    agent: 'hermes', primary: null, related: ['runtime-a', 'runtime-b'],
  });
  call(Command.Link, `${q} --agent hermes --session runtime-c`);
  assert.equal(call(Command.Relations, `--quest ${q}`).agents[1].primary, null);
  call(Command.Primary, q);
  call(Command.Unlink, q);
  assert.equal(call(Command.Unlink, q).changed, false);
  assert.equal(call(Command.Relations, `--quest ${q}`).agents[1].primary, null);
  call(Command.Primary, `${q} --agent claude --clear`, null);
  assert.equal(call(Command.Relations, `--quest ${q}`).agents[0].primary, null);
  assert.equal(call(Command.Relations, '').quests.length, 1);
  const db = new Database(path, { readonly: true });
  assert.equal(db.prepare('PRAGMA integrity_check').pluck().get(), 'ok'); db.close();
});

test('missing quest, CLI errors and malformed arguments change no links', t => {
  const { call, home, path, env, quests: [q] } = fixture(t);
  assert.ok(q);
  assert.ok(call(Command.Link, 'missing').error);
  assert.equal(existsSync(path), false);
  for (const [command, args] of [
    [Command.Link, ''], [Command.Link, `${q} extra`], [Command.Link, `${q} --agent hermes`],
    [Command.Link, `${q} --session s`], [Command.Link, `${q} --agent hermes --agent claude --session s`],
    [Command.Link, `${q} --clear`], [Command.Link, `${q} --agent claude --session bad`],
    [Command.Link, `${q} --unknown x`], [Command.Link, `${q} --session=a`], [Command.Link, '"open'],
    [Command.Primary, `${q} --clear --session s`], [Command.Relations, q],
    [Command.Relations, `--quest ${q} --agent hermes --session s`], [Command.Relations, '--clear'],
  ]) assert.ok(call(command!, args!, null).error, `${command} ${args}`);
  assert.equal(existsSync(path), false);
  call(Command.Link, q);
  const before = readFileSync(path);
  assert.ok(call(Command.Link, 'missing').error);
  assert.deepEqual(readFileSync(path), before);
  const bin = join(home, 'bin'); mkdirSync(bin);
  for (const script of ['#!/bin/sh\nexit 1\n', '#!/bin/sh\nprintf not-json\n', '#!/bin/sh\nprintf \'{"id":"different"}\'\n']) {
    writeFileSync(join(bin, 'fini'), script, { mode: 0o700 });
    assert.ok(call(Command.Link, q, undefined, {}, { ...env, PATH: `${bin}:${env.PATH}` }).error);
    assert.deepEqual(readFileSync(path), before);
  }
});

test('parsed IDs remain exact; runtime and explicit identities stay separate', () => {
  const current = { agent: 'hermes', session_id: ' Current / 漢字 ' };
  const c = new RelationCommand(Command.Link, "' 001 / 漢字 \"quest\" ' --agent other --session ' 00ABC '", current);
  assert.equal(c.quest, ' 001 / 漢字 "quest" ');
  assert.deepEqual(c.session, { agent: 'other', session_id: ' 00ABC ' });
  assert.deepEqual(new RelationCommand(Command.Link, '001', current).session, current);
  assert.equal(new RelationCommand(Command.Primary, '001 --clear', { agent: 'hermes' }).agent, 'hermes');
  for (const session_id of [null, '', ' ', 'a\n', 'a\0']) {
    assert.throws(() => new RelationCommand(Command.Link, '001', { agent: 'hermes', session_id }));
  }
  assert.throws(() => new RelationCommand(Command.Link, '001 --agent Hermes --session a', current));
  assert.equal((execute({ command: Command.Link, raw_args: '001', current, db_path: 'relative' }) as { error: string }).error,
    'Database path must be an absolute file path.');
});

test('SQLite constraints preserve unique links and at most one primary per quest and agent', t => {
  const { path } = fixture(t), repository = new SqliteRelationsRepository(path);
  repository.link('001', { agent: 'hermes', session_id: 'a' });
  repository.link('001', { agent: 'hermes', session_id: 'b' });
  repository.close();
  const db = new Database(path); t.after(() => db.close());
  assert.throws(() => db.prepare('INSERT INTO relations VALUES (?, ?, ?, ?)').run('001', 'hermes', 'a', null), /UNIQUE/);
  assert.throws(() => db.prepare('UPDATE relations SET is_primary=1 WHERE session_id=?').run('b'), /UNIQUE/);
  assert.throws(() => db.prepare('UPDATE relations SET is_primary=0').run(), /CHECK/);
  db.prepare('INSERT INTO relations VALUES (?, ?, ?, ?)').run('001', 'claude', uuid, 1);
  assert.equal(db.prepare('SELECT count(*) FROM relations').pluck().get(), 3);
});

test('independent SQLite writers retain all links and one first primary', async t => {
  const { env, quests: [quest], call } = fixture(t);
  assert.ok(quest);
  const workers = Array.from({ length: 12 }, (_, i) => {
    const child = spawn(process.execPath, [CLI], { env, stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '', err = ''; child.stdout.on('data', v => { out += v; }); child.stderr.on('data', v => { err += v; });
    const done = once(child, 'close').then(([code]) => {
      assert.equal(code, 0, err); assert.equal(JSON.parse(out).error, undefined, out);
    });
    child.stdin.end(JSON.stringify({ command: Command.Link, raw_args: quest,
      current: { agent: i % 2 ? 'hermes' : 'other', session_id: `s-${i % 8}` } }));
    return done;
  });
  await Promise.all(workers);
  const result = call(Command.Relations, `--quest ${quest}`);
  assert.equal(result.agents.length, 2);
  for (const group of result.agents) {
    assert.ok(group.primary); assert.equal(group.related.length, 3);
  }
});

test('session relations expand each quest one level and stop', t => {
  const { call, env, quests: [q, other] } = fixture(t);
  assert.ok(q && other);
  // Two sessions share two quests: a recursive expansion would loop between them.
  call(Command.Link, q); call(Command.Link, other);
  call(Command.Link, `${q} --agent claude --session ${uuid}`);
  call(Command.Link, `${other} --agent claude --session ${uuid}`);
  const listed = call(Command.Relations, '');
  assert.deepEqual(listed.quests, [q, other].sort().map(id => ({
    quest_id: id, title: id === q ? 'first' : 'second', is_primary: true,
    agents: [{ agent: 'claude', primary: uuid, related: [] },
      { agent: 'hermes', primary: 'runtime-a', related: [] }],
  })));
  // Sessions are plain IDs: no quest list inside an agent group, at any depth.
  for (const quest of listed.quests) {
    for (const group of quest.agents) {
      assert.deepEqual(Object.keys(group).sort(), ['agent', 'primary', 'related']);
      for (const id of [group.primary, ...group.related]) assert.equal(typeof id, 'string');
    }
  }
  // The other direction also stops: a quest lists sessions, not their quests.
  assert.equal(JSON.stringify(call(Command.Relations, `--quest ${q}`)).includes('"quests"'), false);
  // A quest that Fini no longer has keeps its link and shows a null title.
  assert.equal(spawnSync('fini', ['quest', 'delete', '--id', other], { env }).status, 0);
  const after = call(Command.Relations, '').quests.find((row: { quest_id: string }) => row.quest_id === other);
  assert.equal(after.title, null);
  assert.equal(after.agents.length, 2);
});

test('link --primary takes the primary and keeps the former one related', t => {
  const { call, env, quests: [q] } = fixture(t);
  assert.ok(q);
  call(Command.Link, q);
  call(Command.Link, `${q} --agent hermes --session runtime-b`);
  const promoted = call(Command.Link, `${q} --agent hermes --session runtime-b --primary`);
  assert.equal(promoted.changed, true);
  assert.deepEqual(promoted.agents, [{ agent: 'hermes', primary: 'runtime-b', related: ['runtime-a'] }]);
  assert.equal(call(Command.Link, `${q} --agent hermes --session runtime-b --primary`).changed, false);
  assert.ok(call(Command.Unlink, `${q} --primary`).error);
  // The model tools send the same option as the true/false field primary.
  const p = spawnSync(process.execPath, [CLI], { env, encoding: 'utf8', input: JSON.stringify({ command: Command.Link,
    input: { quest_id: q, primary: true }, current: { agent: 'hermes', session_id: 'runtime-a' } }) });
  assert.equal(JSON.parse(p.stdout).agents[0].primary, 'runtime-a');
});
