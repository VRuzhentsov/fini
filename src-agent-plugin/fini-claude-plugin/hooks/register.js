// Claude mod adapter: runtime identity and display only. Shared core owns everything else.
const COMMANDS = ['fini:link', 'fini:unlink', 'fini:primary', 'fini:relations'];

export function register(on) {
  for (const command of COMMANDS) {
    on('command.run', { command }, async ($, e) => {
      let result;
      try {
        const request = { command: e.command, raw_args: e.args,
          current: { agent: 'claude', session_id: await $.session.id() } };
        const child = await $.process.run(['node', `${$.plugin.root}/shared/cli.mts`],
          { stdin: JSON.stringify(request), timeoutMs: 20_000 });
        result = JSON.parse(child.stdout);
      } catch (error) {
        result = { error: `Fini core did not answer (${error}); read relations before retrying.` };
      }
      // Answer without next: the markdown body never reaches the model.
      return { text: JSON.stringify(result, null, 2), isTextDisplayOnly: true };
    });
  }
}
