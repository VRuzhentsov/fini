// Claude mod adapter: runtime identity and display only. Shared core owns everything else.
const COMMANDS = ['fini:link', 'fini:unlink', 'fini:primary', 'fini:relations'];

async function core($, command, raw_args) {
  try {
    const request = { command, raw_args, current: { agent: 'claude', session_id: await $.session.id() } };
    const child = await $.process.run(['node', `${$.plugin.root}/shared/cli.mts`],
      { stdin: JSON.stringify(request), timeoutMs: 20_000 });
    return JSON.parse(child.stdout);
  } catch (error) {
    return { error: `Fini core did not answer (${error}); read relations before retrying.` };
  }
}

// The quest note in the system prompt. It changes only when it is read again,
// so the prompt cache stays valid between Fini commands.
let note = Promise.resolve('');
function refresh($) {
  note = core($, 'fini:context', '').then(result => result.text ?? '');
}

export function register(on) {
  on('session.start', ($, e, next) => { refresh($); return next(e); });

  on('prompt.compose', async ($, e, next) => {
    const result = await next(e);
    const text = await note;
    return text ? { sections: [...result.sections, { id: 'fini:quests', text, scope: 'session' }] } : result;
  });

  for (const command of COMMANDS) {
    on('command.run', { command }, async ($, e) => {
      const result = await core($, e.command, e.args);
      refresh($);
      // Answer without next: the markdown body never reaches the model.
      return { text: JSON.stringify(result, null, 2), isTextDisplayOnly: true };
    });
  }
}
