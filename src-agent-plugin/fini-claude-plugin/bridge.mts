#!/usr/bin/env node
// Native context/transport only. No domain or persistence implementation here.
import { Command, execute, errorResult, MAX_BYTES, object } from '../shared/core.mts';

export function handleHook(value: unknown): unknown {
  try {
    if (!object(value) || value.hook_event_name !== 'UserPromptExpansion'
        || value.expansion_type !== 'slash_command' || value.command_source !== 'plugin'
        || !Object.values(Command).includes(value.command_name as typeof Command[keyof typeof Command])
        || typeof value.command_args !== 'string') {
      throw new Error('Expected native Fini UserPromptExpansion context.');
    }
    return execute({ command: value.command_name, raw_args: value.command_args,
      current: { agent: 'claude', session_id: value.session_id } });
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
