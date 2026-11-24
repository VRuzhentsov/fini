import { spawnSync } from 'node:child_process';
import { closeSync, mkdirSync, openSync } from 'node:fs';
import { dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export type Provider = 'hermes' | 'claude';
export type Providers = Partial<Record<Provider, string[]>>;
export type State = { version: 1 | 2; quests: Record<string, Providers> };
export type Operation =
  | { action: 'list'; quest_id: string }
  | { action: 'link'; quest_id: string; provider: Provider; session_id: string };
export type Request = Operation & { state_path: string };
export const MAX_BYTES = 1024 * 1024;
const UUID = /^[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}(?![\s\S])/;
// Match the former Python str.strip() whitespace set, not JS trim() (which
// would reject a literal BOM identifier but accept a whitespace-only NEL).
// eslint-disable-next-line no-control-regex -- Match the Python whitespace characters exactly.
const BLANK = /^[\u0009-\u000d\u001c-\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]*$/;
export const STATE_ERROR = 'Unsupported or unreadable quest session state; refusing to overwrite it.';

export function object(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function exactKeys(value: Record<string, unknown>, keys: string[]): boolean {
  return Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
}
export function questId(value: unknown): string {
  if (typeof value !== 'string' || BLANK.test(value)) {
    throw new Error("Supply a non-empty quest ID as the command's entire argument.");
  }
  return value;
}
function sessionId(provider: Provider, value: unknown): string {
  if (provider === 'claude') {
    if (typeof value !== 'string' || !UUID.test(value)) {
      throw new Error('Claude session ID must be an exact hyphenated UUID; nothing was linked.');
    }
  } else if (typeof value !== 'string' || BLANK.test(value)) {
    throw new Error('Hermes did not supply a current session ID; nothing was linked.');
  }
  return value as string;
}
function operation(value: Record<string, unknown>): Operation {
  if (value.action !== 'list' && value.action !== 'link') throw new Error('Unsupported action.');
  const quest = questId(value.quest_id);
  if (value.action === 'list') return { action: 'list', quest_id: quest };
  if (value.provider !== 'hermes' && value.provider !== 'claude') throw new Error('Unsupported provider.');
  return { action: 'link', quest_id: quest, provider: value.provider,
    session_id: sessionId(value.provider, value.session_id) };
}

// Both the Hermes slash bridge and a future agent adapter use this boundary.
// Commands carry raw arguments; direct adapters carry explicit provider/session input.
export function normalizeRequest(value: unknown): Request {
  if (!object(value)) throw new Error('Supply a JSON request object.');
  if (typeof value.state_path !== 'string' || !isAbsolute(value.state_path)
      || value.state_path.includes('\0')) throw new Error('state_path must be an absolute local file path.');
  let op: Operation;
  if (Object.hasOwn(value, 'command')) {
    if (!exactKeys(value, ['state_path', 'command', 'raw_args', 'session_id'])) {
      throw new Error('Invalid command request fields.');
    }
    switch (value.command) {
      case 'fini-link':
        op = operation({ action: 'link', quest_id: value.raw_args, provider: 'hermes', session_id: value.session_id });
        break;
      case 'fini-sessions':
        op = operation({ action: 'list', quest_id: value.raw_args });
        break;
      case 'fini-link-claude': {
        let args: unknown;
        try { args = typeof value.raw_args === 'string' ? JSON.parse(value.raw_args) : null; }
        catch { throw new Error('Supply JSON with exactly quest_id and session_id.'); }
        if (!object(args) || !exactKeys(args, ['quest_id', 'session_id'])) {
          throw new Error('Supply JSON with exactly quest_id and session_id.');
        }
        op = operation({ action: 'link', provider: 'claude', ...args });
        break;
      }
      default: throw new Error('Unsupported command.');
    }
  } else {
    const keys = value.action === 'list' ? ['state_path', 'action', 'quest_id']
      : ['state_path', 'action', 'quest_id', 'provider', 'session_id'];
    if (!exactKeys(value, keys)) throw new Error('Invalid request fields.');
    op = operation(value);
  }
  return { ...op, state_path: value.state_path };
}

export function parseState(text: string): State {
  try {
    const value: unknown = JSON.parse(text);
    if (!object(value) || !exactKeys(value, ['version', 'quests'])
        || (value.version !== 1 && value.version !== 2) || !object(value.quests)) throw new Error();
    // Null-prototype maps preserve IDs such as __proto__, constructor and toString.
    const quests: Record<string, Providers> = Object.create(null);
    for (const [quest, raw] of Object.entries(value.quests)) {
      questId(quest);
      const providers = value.version === 1 ? { hermes: raw } : raw;
      if (!object(providers) || Object.keys(providers).some(p => p !== 'hermes' && p !== 'claude')) throw new Error();
      const result: Providers = {};
      for (const provider of ['hermes', 'claude'] as const) {
        if (!Object.hasOwn(providers, provider)) continue;
        const ids = providers[provider];
        if (!Array.isArray(ids)) throw new Error();
        result[provider] = ids.map(id => sessionId(provider, id));
      }
      quests[quest] = result;
    }
    return { version: value.version, quests };
  } catch { throw new Error(STATE_ERROR); }
}

export function listing(request: Request, providers: Providers) {
  return { quest_id: request.quest_id, session_ids: providers.hermes ?? [],
    sessions: (['hermes', 'claude'] as const).flatMap(provider =>
      (providers[provider] ?? []).map(session_id => ({ provider, session_id }))),
    state_path: request.state_path };
}

export function errorResult(error: unknown): { error: string } {
  return { error: error instanceof Error ? error.message : String(error) };
}

export function execute(value: unknown): unknown {
  try {
    // Validate before creating even the storage directory or lock.
    const request = normalizeRequest(value);
    const root = dirname(request.state_path);
    mkdirSync(root, { recursive: true, mode: 0o700 });
    // Same stable name and flock(2) protocol as the previous Python writer.
    const lock = join(root, 'sessions.lock');
    closeSync(openSync(lock, 'a', 0o600));
    const result = spawnSync('flock', ['--exclusive', '--timeout', '8', '--conflict-exit-code', '75',
      '--no-fork', lock, process.execPath, fileURLToPath(new URL('./locked-worker.mts', import.meta.url))],
    { input: JSON.stringify(request), encoding: 'utf8', maxBuffer: MAX_BYTES, timeout: 12_000, killSignal: 'SIGKILL' });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(result.status === 75 ? 'Quest session state is busy; nothing was linked.'
        : 'Quest session core failed; inspect the local Node/flock installation.');
    }
    const response: unknown = JSON.parse(result.stdout);
    if (!object(response)) throw new Error('Invalid core response.');
    return response;
  } catch (error) { return errorResult(error); }
}
