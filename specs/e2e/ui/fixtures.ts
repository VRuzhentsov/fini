/**
 * Fixtures for real-app UI e2e via tauri-plugin-playwright.
 *
 * Local runs show the real Fini window. CI runs the same suite under xvfb.
 * The app state is isolated under FINI_APP_DATA_DIR so tests do not touch the
 * user's normal data directory.
 */
import { createTauriTest } from '@srsholmes/tauri-playwright';

const tauriCommand = process.env.FINI_BINARY
  ? `xvfb-run -a env FINI_APP_DATA_DIR=/var/tmp/fini-e2e-ui TZ=UTC ${process.env.FINI_BINARY} app`
  : 'env FINI_APP_DATA_DIR=/var/tmp/fini-e2e-ui TZ=UTC npx tauri dev --features e2e-testing -- -- app';

export const { test, expect } = createTauriTest({
  tauriCommand,
  tauriCwd: process.cwd(),
  mcpSocket: '/var/tmp/fini-playwright.sock',
  startTimeout: 240,
});
