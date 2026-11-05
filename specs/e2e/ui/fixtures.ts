/**
 * Fixtures for real-app UI e2e via tauri-plugin-playwright.
 *
 * Designed to run alongside `make dev` without resource conflict:
 * - Spawned binary uses an isolated FINI_APP_DATA_DIR.
 * - Discovery and sync-WS ports default to randomized values in the 47000-47999
 *   range (above the 46000-base allocation of the multi-device projects), so the e2e
 *   binary does not bind ports the live dev app already holds.
 * - When FINI_APP_BINARY points at a prebuilt e2e binary, no `tauri dev` is started,
 *   so Vite port 1420 is free for the developer's `make dev` session.
 * - The app draws on DISPLAY: headful on the real session (`make e2e`),
 *   headless on the lane's Xvfb (DISPLAY=:99) in the e2e container.
 * - Each test starts its own app (the `tauriPage` fixture is test-scoped) and
 *   stops it with SIGTERM at the end of the test.
 */
import { createTauriTest } from '@srsholmes/tauri-playwright';
import { mkdirSync } from 'fs';
import { join } from 'path';

function pickRandomPort(envName: string, base: number, span: number): string {
  const fromEnv = process.env[envName];
  if (fromEnv) return fromEnv;
  return String(base + Math.floor(Math.random() * span));
}

export const e2eUiRoot = process.env.FINI_E2E_ROOT ?? join(process.cwd(), 'tmp', 'fini-e2e-ui');
mkdirSync(e2eUiRoot, { recursive: true });
const dataDir = process.env.FINI_APP_DATA_DIR ?? e2eUiRoot;
const discoveryPort = pickRandomPort('FINI_DISCOVERY_PORT', 47000, 500);
const wsPort = pickRandomPort('FINI_SPACE_SYNC_WS_PORT', 47500, 500);
const envFlags = `FINI_APP_DATA_DIR=${dataDir} FINI_DISCOVERY_PORT=${discoveryPort} FINI_SPACE_SYNC_WS_PORT=${wsPort} TZ=UTC`;
// A backstop against an app process that outlives its test -- not a
// budget for the test. Each test starts its own app; when this fires
// during a test, the app simply disappears, and the test hangs until its
// own timeout, since a dead socket neither answers nor times out.
//
// That is exactly what 300s did once the lane grew past five minutes --
// whichever test happened to run around the four-minute mark died with a
// bare 180s timeout and no error, and the one after it inherited a wedged
// app. The failure moved with timing, which made it read as flake for a
// long time.
//
// 1800s leaves room for the lane to keep growing while still killing a
// genuinely stuck process well inside CI's own limits.
const binaryTimeout = process.platform === 'linux' ? '/usr/bin/timeout --foreground --kill-after=5s 1800s ' : '';

const tauriCommand = process.env.FINI_APP_BINARY
  ? `${binaryTimeout}env ${envFlags} ${process.env.FINI_APP_BINARY}`
  :`env ${envFlags} npm run tauri -- dev --features ui-plane,devtools`;

export const { test, expect } = createTauriTest({
  tauriCommand,
  tauriCwd: process.cwd(),
  mcpSocket: join(e2eUiRoot, 'fini-playwright.sock'),
  startTimeout: 240,
});
