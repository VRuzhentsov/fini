// Real native Claude ZIP loading with a dead model endpoint and isolated data.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
assert.ok(process.env.TMPDIR);
const home = mkdtempSync(join(process.env.TMPDIR, 'fini-native-'));
function command(executable: string, args: string[], env: NodeJS.ProcessEnv, cwd = root) {
  const p = spawnSync(executable, args, { env, cwd, encoding: 'utf8', timeout: 60_000, maxBuffer: 4 * 1024 * 1024 });
  assert.equal(p.error, undefined); assert.equal(p.status, 0, p.stderr || p.stdout);
  return p.stdout;
}
try {
  const env: NodeJS.ProcessEnv = { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR,
    HOME: home, CLAUDE_CONFIG_DIR: join(home, 'claude'), HERMES_HOME: join(home, 'hermes'),
    XDG_DATA_HOME: join(home, 'data'), XDG_CONFIG_HOME: join(home, 'config'),
    ANTHROPIC_API_KEY: 'isolated-fixture-not-a-real-key', ANTHROPIC_BASE_URL: 'http://127.0.0.1:1',
    DISABLE_AUTOUPDATER: '1', DISABLE_TELEMETRY: '1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1' };
  // The plugin folder as an install copies it: its files only, no node_modules and no repository.
  const plugin = join(root, 'agent'), zip = join(home, 'fini.zip');
  command('python3', ['-c', 'import pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); z=zipfile.ZipFile(sys.argv[2],"w",zipfile.ZIP_DEFLATED); [z.write(f,f.relative_to(p)) for f in p.rglob("*") if f.is_file() and "__pycache__" not in f.parts]; z.close()', plugin, zip], env);
  const quest = JSON.parse(command('fini', ['quest', 'create', '--title', 'Native plugin check', '--json'], env)).id;
  const work = join(home, 'work'); mkdirSync(work);
  const common = ['--plugin-dir', zip, '--setting-sources', '', '--strict-mcp-config',
    '--mcp-config', '{"mcpServers":{}}', '--tools', '', '--output-format', 'json'];
  let invocations = 0;
  function native(prompt: string, resume?: string, expectError = false) {
    const result = JSON.parse(command('claude', [...common, ...(resume ? ['--resume', resume] : []), '-p', prompt], env, work));
    assert.equal(result.is_error, false);
    assert.equal(result.num_turns, 0); assert.equal(result.total_cost_usd, 0); assert.equal(result.duration_api_ms, 0);
    assert.doesNotMatch(result.result, /blocked by hook/);
    assert.match(result.result, /^fini: \{/);
    const body = JSON.parse(result.result.slice('fini: '.length));
    assert.equal(typeof body.error === 'string', expectError, JSON.stringify(body)); invocations++;
    return { result, body };
  }
  const first = native(`/fini:link ${quest}`), sid = first.result.session_id;
  assert.equal(first.body.session_id, sid); assert.equal(first.body.changed, true);
  assert.equal(first.body.db_path, join(home, '.fini', 'agent-sessions.db'));
  assert.equal(native(`/fini:link ${quest}`, sid).body.changed, false);
  const second = native(`/fini:link ${quest}`), other = second.result.session_id;
  assert.notEqual(other, sid); assert.equal(second.body.session_id, other);
  const listed = native(`/fini:relations --quest ${quest}`).body;
  assert.deepEqual(listed.agents, [{ agent: 'claude', primary: sid, related: [other] }]);
  assert.equal(native('/fini:relations', sid).body.quests.length, 1);
  assert.equal(native(`/fini:link ${quest} --primary`, other).body.agents[0].primary, other);
  assert.equal(native(`/fini:unlink ${quest}`, other).body.changed, true);
  native('/fini:link missing-quest', sid, true);
  native(`/fini:link ${quest} --agent hermes`, sid, true);
  assert.equal(native(`/fini:link ${quest} --agent hermes --session explicit-native`, sid).body.agent, 'hermes');
  assert.equal(native('/fini:relations --agent hermes --session explicit-native').body.quests.length, 1);
  assert.equal(existsSync(join(home, 'hermes', 'plugin-data')), false);
  console.log(JSON.stringify({ verified: 'mod command.run answers for native /fini:* commands, display-only output, ZIP install copy with node:sqlite and no npm packages, runtime IDs, resume, primary, unlink, both relation directions, CLI failure and invalid options',
    version: command('claude', ['--version'], env, work).trim(), invocations, model_turns: 0, api_ms: 0, cost_usd: 0 }));
} finally { rmSync(home, { recursive: true, force: true }); }
