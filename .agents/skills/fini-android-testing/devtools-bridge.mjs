// Thin bridge to a running Fini debug app's devtools. No verbs, no wrapped
// actions -- it connects, evaluates whatever expression it is given inside
// the app's webview, prints the result, exits.
//
//   node .claude/skills/fini-android-testing/devtools-bridge.mjs <port> "<js>"
//
// Run it from the repo root -- it resolves `@srsholmes/tauri-playwright`
// from the repo's node_modules.
//
// Ports: desktop 9224, phone 9223 (needs `adb forward tcp:9223 tcp:9223`).
//
// The expression runs in the webview, so the DOM and Tauri are both in
// scope. `invoke(cmd, args)` is provided as a shorthand. The last expression
// is the result; use an async IIFE when you need `await`.
//
//   "document.querySelectorAll('button').length"
//   "[...document.querySelectorAll('button')].map(b => b.textContent)"
//   "[...document.querySelectorAll('button')].find(b => b.textContent.includes('Pair')).click()"
//   "invoke('device_connection_get_paired_devices')"
//   "(async () => { const p = await invoke('device_connection_get_paired_devices');
//                   for (const d of p) await invoke('device_connection_unpair',
//                     { peerDeviceId: d.peer_device_id }); return p.length; })()"

import { PluginClient, TauriPage } from '@srsholmes/tauri-playwright';

const [, , portArg, ...exprParts] = process.argv;
const port = Number(portArg);
const expr = exprParts.join(' ');
if (!port || !expr) {
  console.error(
    'usage: node .claude/skills/fini-android-testing/devtools-bridge.mjs <port> "<javascript>"',
  );
  process.exit(2);
}

const client = new PluginClient(undefined, port);
await client.connect();
if (!(await client.send({ type: 'ping' })).ok) {
  console.error(`no Fini devtools on port ${port}`);
  process.exit(1);
}
const page = new TauriPage(client);

try {
  const result = await page.evaluate(`(async () => {
    const invoke = (cmd, args = {}) => {
      const i = window.__TAURI_INTERNALS__?.invoke;
      if (!i) throw new Error('Tauri invoke unavailable');
      return i(cmd, args);
    };
    return await (${expr});
  })()`);
  console.log(typeof result === 'string' ? result : JSON.stringify(result, null, 2));
} catch (err) {
  console.error(`FAILED: ${err?.message ?? err}`);
  process.exitCode = 1;
} finally {
  await client.disconnect?.();
  process.exit(process.exitCode ?? 0);
}
