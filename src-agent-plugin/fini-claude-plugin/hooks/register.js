// Claude mod adapter: runtime identity and display only. Shared core owns everything else.
const COMMANDS = ['fini:link', 'fini:unlink', 'fini:primary', 'fini:relations'];

// Model tools. The model calls each one as mcp__fini__<name>. The mod adds the
// current session ID from the runtime; the model can only give an explicit target.
const TARGET = {
  agent: { type: 'string', description: 'Agent of an explicit target session. Give it with session_id, or give neither.' },
  session_id: { type: 'string', description: 'ID of an explicit target session. Leave out agent and session_id for the current session.' },
};
const QUEST = { type: 'string', description: 'Fini quest ID.' };
const TOOLS = [
  { name: 'fini_link', command: 'fini:link',
    description: 'Link a Fini quest to the current agent session, or to an explicit agent session. Only link when the user asks or agrees. Quest content changes stay with the fini CLI.',
    inputSchema: { type: 'object', properties: { quest_id: QUEST, ...TARGET }, required: ['quest_id'], additionalProperties: false } },
  { name: 'fini_unlink', command: 'fini:unlink',
    description: 'Remove one link between a Fini quest and the current agent session, or an explicit agent session.',
    inputSchema: { type: 'object', properties: { quest_id: QUEST, ...TARGET }, required: ['quest_id'], additionalProperties: false } },
  { name: 'fini_relations', command: 'fini:relations',
    description: 'Show the Fini quests of the current agent session, or with quest_id the agent sessions of that quest.',
    inputSchema: { type: 'object', properties: { quest_id: QUEST }, additionalProperties: false } },
];

// `args` is command text (raw_args) or tool fields (input).
async function core($, command, args) {
  try {
    const request = { command, [typeof args === 'string' ? 'raw_args' : 'input']: args,
      current: { agent: 'claude', session_id: await $.session.id() } };
    const child = await $.process.run(['node', `${$.plugin.root}/shared/cli.mts`],
      { stdin: JSON.stringify(request), timeoutMs: 20_000 });
    return JSON.parse(child.stdout);
  } catch (error) {
    return { error: `Fini core did not answer (${error}); read relations before retrying.` };
  }
}

// Give the model the quest note as a plugin note in the conversation. A higher-tier
// plugin can skip a prompt.compose hook, but a call on $ reaches the session.
async function tell($) {
  try {
    const { text } = await core($, 'fini:context', '');
    if (text) await $.session.append({ message: { type: 'user', content: [{ type: 'text', text }] } });
  } catch { /* The note is help only; a failure must not stop the session or the command. */ }
}

async function start($) {
  for (const { name, description, inputSchema } of TOOLS) {
    await $.tool.register({ name, description, inputSchema, isDeferred: false });
  }
  await tell($);
}

export function register(on) {
  on('session.start', async ($, e, next) => { await start($); return next(e); });

  for (const command of COMMANDS) {
    on('command.run', { command }, async ($, e) => {
      const result = await core($, e.command, e.args);
      if (command !== 'fini:relations') await tell($);
      // Answer without next: the markdown body never reaches the model.
      return { text: JSON.stringify(result, null, 2), isTextDisplayOnly: true };
    });
  }

  for (const { name, command } of TOOLS) {
    on('tool.run', { tool: `mcp__fini__${name}` }, async ($, e) => {
      // Only the schema fields go to the core; the session ID comes from the runtime.
      const input = Object.fromEntries(['quest_id', 'agent', 'session_id'].filter(key => key in e).map(key => [key, e[key]]));
      const result = await core($, command, input);
      if (command !== 'fini:relations') await tell($);
      return { result: JSON.stringify(result, null, 2) };
    });
  }
}
