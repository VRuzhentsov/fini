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
 * - FINI_E2E_HEADFUL=1 skips the `xvfb-run` wrapper so the test window draws on
 *   the real X session (default for `make e2e`). CI sets FINI_E2E_HEADFUL=0 (or
 *   leaves it unset) to keep the xvfb wrapper.
 */
import { createTauriTest } from '@srsholmes/tauri-playwright';

function pickRandomPort(envName: string, base: number, span: number): string {
  const fromEnv = process.env[envName];
  if (fromEnv) return fromEnv;
  return String(base + Math.floor(Math.random() * span));
}

const dataDir = process.env.FINI_APP_DATA_DIR ?? '/var/tmp/fini-e2e-ui';
const discoveryPort = pickRandomPort('FINI_DISCOVERY_PORT', 47000, 500);
const wsPort = pickRandomPort('FINI_SPACE_SYNC_WS_PORT', 47500, 500);
const headful = process.env.FINI_E2E_HEADFUL === '1';

const envFlags = `FINI_APP_DATA_DIR=${dataDir} FINI_DISCOVERY_PORT=${discoveryPort} FINI_SPACE_SYNC_WS_PORT=${wsPort} TZ=UTC`;

const tauriCommand = process.env.FINI_APP_BINARY
  ? (headful
      ? `env ${envFlags} ${process.env.FINI_APP_BINARY}`
      : `xvfb-run -a env ${envFlags} ${process.env.FINI_APP_BINARY}`)
  : `env ${envFlags} npx tauri dev --features ui-plane,cli-plane,e2e-testing`;

export const { test, expect } = createTauriTest({
  tauriCommand,
  tauriCwd: process.cwd(),
  mcpSocket: '/var/tmp/fini-playwright.sock',
  startTimeout: 240,
});
