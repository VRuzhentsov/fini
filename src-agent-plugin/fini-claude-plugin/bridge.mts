#!/usr/bin/env node
// Native context/transport only. No domain or persistence implementation here.
import { homedir } from 'node:os';
import { isAbsolute, join } from 'node:path';
import { execute, errorResult, MAX_BYTES, object } from '../shared/core.mts';

export function statePath(env: NodeJS.ProcessEnv): string {
  // Shared explicit opt-in wins. Hermes resolves its own profile when unset.
  const explicit = env.FINI_SESSIONS_PATH;
  if (explicit !== undefined) {
    if (!isAbsolute(explicit) || explicit.includes('\0')) throw new Error('FINI_SESSIONS_PATH must be an absolute file path.');
    return explicit;
  }
  if ((env.HERMES_PROFILE && env.HERMES_PROFILE !== 'default') || env.HERMES_DATA_DIR_SUFFIX) {
    throw new Error('Non-default Hermes context: set FINI_SESSIONS_PATH explicitly; profiles are never inferred or merged.');
  }
  const home = env.HERMES_HOME ?? join(homedir(), '.hermes');
  if (!isAbsolute(home) || home.includes('\0')) throw new Error('HERMES_HOME must be absolute, or set FINI_SESSIONS_PATH.');
  return join(home, 'plugin-data', 'fini-hermes-plugin', 'sessions.json');
}

export function handleHook(value: unknown, env = process.env): unknown {
  try {
    if (!object(value) || value.hook_event_name !== 'UserPromptExpansion'
        || value.expansion_type !== 'slash_command' || value.command_source !== 'plugin'
        || (value.command_name !== 'fini:link' && value.command_name !== 'fini:sessions')
        || typeof value.command_args !== 'string') {
      throw new Error('Expected native fini UserPromptExpansion context; nothing was linked.');
    }
    // Never accept a session argument, transcript lookup, env ID or model substitution.
    // The core validates the UUID and the entire literal quest argument before I/O.
    return execute(value.command_name === 'fini:link'
      ? { action: 'link', provider: 'claude', session_id: value.session_id,
          quest_id: value.command_args, state_path: statePath(env) }
      : { action: 'list', quest_id: value.command_args, state_path: statePath(env) });
  } catch (error) { return errorResult(error); }
}

// Block expansion on success AND failure: render deterministic JSON directly to
// the user, never send a bridge instruction to the model. EOF is required.
const timer = setTimeout(() => {
  printResult({ error: 'Claude hook input timed out; nothing was linked.' });
  process.exit(0);
}, 15_000);
function printResult(result: unknown): void {
  const reason = JSON.stringify(result);
  let output = JSON.stringify({ decision: 'block', reason });
  if (Buffer.byteLength(output) > MAX_BYTES) {
    output = JSON.stringify({ decision: 'block', reason: JSON.stringify({
      error: 'Bridge result exceeds 1 MiB; inspect state before retrying.' }) });
  }
  process.stdout.write(`${output}\n`);
}
try {
  let size = 0;
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    const bytes = Buffer.from(chunk);
    size += bytes.length;
    if (size > MAX_BYTES) throw new Error('Claude hook input exceeds 1 MiB; nothing was linked.');
    chunks.push(bytes);
  }
  clearTimeout(timer);
  const text = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks));
  printResult(handleHook(JSON.parse(text)));
} catch (error) { printResult(errorResult(error)); }
finally { clearTimeout(timer); }
