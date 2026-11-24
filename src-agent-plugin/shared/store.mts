import { closeSync, constants, fsyncSync, openSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { dirname, join } from 'node:path';
import { TextDecoder } from 'node:util';
import { listing, parseState } from './core.mts';
import type { Request, State } from './core.mts';

export function readState(path: string): State {
  try {
    const bytes = readFileSync(path);
    // Fail on invalid UTF-8 instead of silently replacing identifiers.
    return parseState(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return { version: 2, quests: Object.create(null) };
    }
    throw error;
  }
}

// The injectable rename seam is for deterministic replacement-failure verification.
export function writeState(path: string, state: State, replace = renameSync): void {
  const root = dirname(path);
  const temporary = join(root, `.sessions-${randomUUID()}.json`);
  try {
    const fd = openSync(temporary, 'wx', 0o600);
    try {
      writeFileSync(fd, `${JSON.stringify(state, null, 2)}\n`, 'utf8');
      fsyncSync(fd);
    } finally { closeSync(fd); }
    replace(temporary, path);
    const directory = openSync(root, constants.O_RDONLY | constants.O_DIRECTORY);
    try { fsyncSync(directory); } finally { closeSync(directory); }
  } finally {
    try { unlinkSync(temporary); }
    // eslint-disable-next-line no-unsafe-finally -- Keep cleanup errors visible to the caller.
    catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
  }
}

// INTERNAL: only called inside util-linux flock's no-fork child.
// Every public store entry point goes through execute() in core.mts.
export function transaction(request: Request): unknown {
  const state = readState(request.state_path);
  const providers = state.quests[request.quest_id] ?? {};
  if (request.action === 'list') return listing(request, providers);
  state.quests[request.quest_id] = providers;
  const ids = providers[request.provider] ??= [];
  const added = !ids.includes(request.session_id);
  if (added) ids.push(request.session_id);
  if (added || state.version === 1) {
    state.version = 2;
    writeState(request.state_path, state);
  }
  return { ...listing(request, providers), provider: request.provider,
    session_id: request.session_id, added };
}
