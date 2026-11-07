// The folder that holds all e2e run folders, and the phone group state file.

import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..');

export const RUN_ROOT = process.env.FINI_E2E_ROOT
  ?? process.env.FINI_E2E_CI_RESULTS_DIR
  ?? join(REPO_ROOT, 'tmp', 'fini-e2e-multi-device');
