// Optional real Claude CLI acceptance probe. No user credentials/model endpoint.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = dirname(dirname(dirname(fileURLToPath(import.meta.url))));
const home = mkdtempSync(join(tmpdir(), 'fini-native-'));
function command(executable: string, args: string[], env?: NodeJS.ProcessEnv, cwd = root) {
  const p = spawnSync(executable, args, { env, cwd, encoding: 'utf8', timeout: 45_000, maxBuffer: 4 * 1024 * 1024 });
  assert.equal(p.error, undefined); assert.equal(p.status, 0, p.stderr || p.stdout);
  return p.stdout;
}
try {
  command(process.execPath, [join(root, 'fini-claude-plugin', 'package.mts')]);
  const artifact = join(root, '.artifacts', 'claude', 'fini'), zip = join(home, 'fini.zip');
  // Python is used only to construct a standard ZIP for this optional test.
  command('python3', ['-c', 'import pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); z=zipfile.ZipFile(sys.argv[2],"w"); [z.write(f,f.relative_to(p)) for f in p.rglob("*") if f.is_file()]; z.close()', artifact, zip]);
  const env: NodeJS.ProcessEnv = { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR,
    HOME: home, CLAUDE_CONFIG_DIR: join(home, 'claude'), HERMES_HOME: join(home, 'hermes'),
    ANTHROPIC_API_KEY: 'isolated-fixture-not-a-real-key', ANTHROPIC_BASE_URL: 'http://127.0.0.1:1',
    DISABLE_AUTOUPDATER: '1', DISABLE_TELEMETRY: '1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1' };
  const work = join(home, 'work'); mkdirSync(work);
  const common = ['--plugin-dir', zip, '--setting-sources', '', '--strict-mcp-config',
    '--mcp-config', '{"mcpServers":{}}', '--tools', '', '--output-format', 'json'];
  function native(prompt: string, resume?: string) {
    const result = JSON.parse(command('claude', [...common, ...(resume ? ['--resume', resume] : []), '-p', prompt], env, work));
    assert.equal(result.is_error, false);
    assert.equal(result.num_turns, 0); assert.equal(result.total_cost_usd, 0); assert.equal(result.duration_api_ms, 0);
    assert.match(result.result, /UserPromptExpansion operation blocked by hook/);
    const body = JSON.parse(result.result.split('\n')[1]); assert.equal(body.error, undefined);
    return { result, body };
  }
  const first = native('/fini:link 000-native-probe');
  assert.equal(first.body.session_id, first.result.session_id); assert.equal(first.body.added, true);
  const path = join(home, 'hermes', 'plugin-data', 'fini-hermes-plugin', 'sessions.json');
  assert.equal(first.body.state_path, path);
  const repeat = native('/fini:link 000-native-probe', first.result.session_id);
  assert.equal(repeat.body.session_id, first.result.session_id); assert.equal(repeat.body.added, false);
  const second = native('/fini:link 000-native-probe');
  assert.notEqual(second.result.session_id, first.result.session_id);
  assert.equal(second.body.session_id, second.result.session_id);
  const before = readFileSync(path);
  const listed = native('/fini:sessions 000-native-probe');
  assert.deepEqual(listed.body.sessions, [
    { provider: 'claude', session_id: first.result.session_id },
    { provider: 'claude', session_id: second.result.session_id },
  ]);
  assert.deepEqual(readFileSync(path), before);
  assert.deepEqual(JSON.parse(before.toString()).quests['000-native-probe'].claude,
    [first.result.session_id, second.result.session_id]);
  console.log(JSON.stringify({ verified: 'real Claude ZIP load, native expansion hook, generated runtime IDs, resume dedup, multi-session listing, readback',
    version: command('claude', ['--version'], env, work).trim(), model_turns: 0, api_ms: 0, cost_usd: 0 }));
} finally { rmSync(home, { recursive: true, force: true }); }
