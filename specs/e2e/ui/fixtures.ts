/**
 * Fixtures for real-app UI e2e via tauri-plugin-playwright.
 *
 * Designed to run alongside `make dev` without resource conflict:
 * - Spawned binary uses an isolated FINI_APP_DATA_DIR.
 * - Discovery and sync-WS ports default to randomized values in the 47000-47999
 *   range (above the multi-actor lane's 46000-base allocation), so the e2e
 *   binary does not bind ports the live dev app already holds.
 * - When FINI_APP_BINARY points at a prebuilt e2e binary, no `tauri dev` is started,
 *   so Vite port 1420 is free for the developer's `make dev` session.
 * - The app starts directly on the existing display: the real X session for
 *   `make e2e` (FINI_E2E_HEADFUL=1), the lane's Xvfb (DISPLAY=:99) in the e2e
 *   container. Only with no display at all is it wrapped in `xvfb-run`.
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
// The app needs an xvfb-run wrapper only when no display exists. The e2e
// container runs one Xvfb for the whole lane and sets DISPLAY=:99; a local
// `make e2e` draws on the real session. With a display, the app starts
// directly, so `timeout` sends the fixture's SIGTERM to fini-app itself
// (`env` replaces itself with the app). Under xvfb-run the signal stops at
// xvfb-run's shell: fini-app and its Xvfb stayed alive after each test,
// one app per test, until the container's process limit (podman: 2048)
// stopped new apps from starting.
const useXvfbRun = process.env.FINI_E2E_HEADFUL !== '1' && !process.env.DISPLAY;

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
  ? (useXvfbRun
      ? `${binaryTimeout}xvfb-run -a env ${envFlags} ${process.env.FINI_APP_BINARY}`
      : `${binaryTimeout}env ${envFlags} ${process.env.FINI_APP_BINARY}`)
  : `env ${envFlags} npm run tauri -- dev --features ui-plane,devtools`;

export const { test, expect } = createTauriTest({
  tauriCommand,
  tauriCwd: process.cwd(),
  mcpSocket: join(e2eUiRoot, 'fini-playwright.sock'),
  startTimeout: 240,
});
