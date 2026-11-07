// Stops the virtual phones after the run, only if a test started them
// (`android-phones.json` exists). FINI_E2E_KEEP_PHONES=1 keeps them on for the
// next local run.

import { PhoneGroup } from './phones/phone-group.ts';

export default async function globalTeardown(): Promise<void> {
  if (process.env.FINI_E2E_KEEP_PHONES === '1') return;
  await PhoneGroup.stopRunning();
}
