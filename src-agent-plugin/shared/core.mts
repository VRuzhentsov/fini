import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { isAbsolute, join } from 'node:path';
import { SqliteRelationsRepository } from './store.mts';
import type { RelationsRepository, Session } from './store.mts';

export const MAX_BYTES = 1024 * 1024;
export const Command = {
  Link: 'fini:link', Unlink: 'fini:unlink', Primary: 'fini:primary', Relations: 'fini:relations',
} as const;
type CommandName = typeof Command[keyof typeof Command];

export function object(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
export function errorResult(error: unknown): { error: string } {
  return { error: error instanceof Error ? error.message : String(error) };
}

// One codec owns command syntax and literal identifier checks for both agents.
export class RelationCommand {
  command: CommandName;
  quest?: string;
  session?: Session;
  agent?: string;
  clear: boolean;

  static identifier(value: unknown): string {
    if (typeof value !== 'string' || !value.trim() || value.startsWith('-')
        || /[\p{Cc}\p{Cs}]/u.test(value)) throw new Error('Supply a non-empty ID without control characters or a leading hyphen.');
    return value;
  }

  static agentName(value: unknown): string {
    if (typeof value !== 'string' || !/^[a-z][a-z0-9_-]*(?![\s\S])/.test(value)) {
      throw new Error('Agent name must start with a lowercase letter and contain only lowercase letters, digits, hyphens or underscores.');
    }
    return value;
  }

  static session(value: unknown): Session {
    if (!object(value) || Object.keys(value).length !== 2) throw new Error('Current runtime session is not available.');
    const agent = this.agentName(value.agent), session_id = this.identifier(value.session_id);
    if (agent === 'claude' && !/^[a-fA-F0-9]{8}(?:-[a-fA-F0-9]{4}){3}-[a-fA-F0-9]{12}$/.test(session_id)) {
      throw new Error('Claude session ID must be an exact hyphenated UUID.');
    }
    return { agent, session_id };
  }

  static tokens(raw: unknown): string[] {
    if (typeof raw !== 'string') throw new Error('Supply command arguments as text.');
    const tokens: string[] = [];
    let value = '', quote = '', active = false;
    for (let i = 0; i < raw.length; i++) {
      const char = raw[i]!;
      if (char === '\\' && quote !== "'") {
        if (i + 1 === raw.length) throw new Error('Incomplete escape in command arguments.');
        value += raw[++i]; active = true;
      } else if (quote) {
        if (char === quote) quote = ''; else value += char;
      } else if (char === '"' || char === "'") { quote = char; active = true;
      } else if (/\s/.test(char)) {
        if (active) { tokens.push(value); value = ''; active = false; }
      } else { value += char; active = true; }
    }
    if (quote) throw new Error('Close the quote in command arguments.');
    if (active) tokens.push(value);
    return tokens;
  }

  constructor(command: unknown, raw: unknown, current: unknown) {
    if (!Object.values(Command).includes(command as CommandName)) throw new Error('Unsupported command.');
    this.command = command as CommandName;
    const tokens = RelationCommand.tokens(raw), positional: string[] = [], options = new Map<string, string | true>();
    for (let i = 0; i < tokens.length; i++) {
      const token = tokens[i]!;
      if (!token.startsWith('-')) { positional.push(token); continue; }
      if (!['--agent', '--session', '--quest', '--clear'].includes(token) || options.has(token)) {
        throw new Error('Unknown or repeated option.');
      }
      if (token === '--clear') { options.set(token, true); continue; }
      const value = tokens[++i];
      if (value === undefined || value.startsWith('-')) throw new Error('Each option needs one value.');
      options.set(token, value);
    }
    this.clear = options.has('--clear');
    const agent = options.get('--agent'), session = options.get('--session'), quest = options.get('--quest');
    if (this.command === Command.Relations) {
      if (positional.length || this.clear || (quest !== undefined && options.size !== 1)) {
        throw new Error('Use relations with no arguments, --quest ID, or --agent NAME --session ID.');
      }
      if (quest !== undefined) { this.quest = RelationCommand.identifier(quest); return; }
    } else {
      if (positional.length !== 1 || quest !== undefined || (this.clear && this.command !== Command.Primary)) {
        throw new Error('Supply one quest ID and only the supported target options.');
      }
      this.quest = RelationCommand.identifier(positional[0]);
      if (this.clear) {
        if (session !== undefined) throw new Error('--clear does not accept --session.');
        // Clearing needs the current agent, not a current session ID.
        this.agent = RelationCommand.agentName(agent ?? (object(current) ? current.agent : undefined));
        return;
      }
    }
    if ((agent !== undefined) !== (session !== undefined)) throw new Error('Supply --agent and --session together.');
    this.session = RelationCommand.session(agent === undefined ? current : { agent, session_id: session });
  }

  run(repository: RelationsRepository) {
    if (this.command === Command.Relations && this.quest === undefined) {
      // Show each linked quest with the sessions of every agent, not only this one.
      return { ...this.session!, quests: repository.quests(this.session!).map(row => ({
        quest_id: row.quest_id, is_primary: row.is_primary === true,
        agents: RelationCommand.agents(repository, row.quest_id) })) };
    }
    const quest = this.quest!;
    let changed: boolean | undefined;
    if (this.command === Command.Link) changed = repository.link(quest, this.session!);
    if (this.command === Command.Unlink) changed = repository.unlink(quest, this.session!);
    if (this.command === Command.Primary) {
      if (this.clear) repository.clear(quest, this.agent!); else repository.promote(quest, this.session!);
    }
    return { quest_id: quest, ...(this.session ?? {}), ...(changed === undefined ? {} : { changed }),
      agents: RelationCommand.agents(repository, quest) };
  }

  static agents(repository: RelationsRepository, quest: string) {
    const groups = new Map<string, { agent: string; primary: string | null; related: string[] }>();
    for (const row of repository.sessions(quest)) {
      const group = groups.get(row.agent) ?? { agent: row.agent, primary: null, related: [] };
      if (row.is_primary) group.primary = row.session_id; else group.related.push(row.session_id);
      groups.set(row.agent, group);
    }
    return [...groups.values()];
  }
}

// Fini is accessed only through its CLI. Returns null when Fini cannot get the exact quest.
function getQuest(id: string): Record<string, unknown> | null {
  const result = spawnSync('fini', ['quest', 'get', '--id', id, '--json'], {
    encoding: 'utf8', timeout: 8000, maxBuffer: MAX_BYTES, killSignal: 'SIGKILL',
  });
  if (result.error || result.status !== 0) return null;
  try {
    const quest: unknown = JSON.parse(result.stdout);
    return object(quest) && quest.id === id ? quest : null;
  } catch { return null; }
}

export function execute(value: unknown): unknown {
  try {
    if (!object(value) || Object.keys(value).some(key => !['command', 'raw_args', 'current', 'db_path'].includes(key))) {
      throw new Error('Supply only command, raw_args, current and optional db_path.');
    }
    const command = new RelationCommand(value.command, value.raw_args, value.current);
    const path = value.db_path ?? process.env.FINI_AGENT_SESSIONS_DB ?? join(homedir(), '.fini', 'agent-sessions.db');
    if (typeof path !== 'string' || !isAbsolute(path) || path.includes('\0')) throw new Error('Database path must be an absolute file path.');
    if (command.command === Command.Link && !getQuest(command.quest!)) {
      // Validate before any plugin DB I/O.
      throw new Error('Fini could not get this quest; no link was changed.');
    }
    const repository: RelationsRepository = new SqliteRelationsRepository(path);
    let result;
    try { result = command.run(repository); }
    finally { repository.close(); }
    if ('quests' in result) {
      // A deleted or unreadable quest keeps its link and shows a null title.
      result = { ...result, quests: result.quests.map(({ quest_id, ...rest }) => {
        const title = getQuest(quest_id)?.title;
        return { quest_id, title: typeof title === 'string' ? title : null, ...rest };
      }) };
    }
    return { ...result, db_path: path };
  } catch (error) { return errorResult(error); }
}
