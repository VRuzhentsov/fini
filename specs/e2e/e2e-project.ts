// The names of the Playwright projects and of the environment variables that
// more than one file reads. No runtime imports, so scripts can import it.

export const E2eProject = {
  Cli: 'cli',
  Ui: 'ui',
  MultiDevice: 'multi-device',
  DeviceSupport: 'device-support',
} as const;
export type E2eProject = (typeof E2eProject)[keyof typeof E2eProject];

/** Playwright test tags (`{ tag: E2eTag.Smoke }`); select them with `--grep`. */
export const E2eTag = {
  /** A quick check that the harness and the app answer: `npm run test:e2e:smoke`. */
  Smoke: '@smoke',
} as const;

export const E2eEnv = {
  /** Test devices in place of the ones of the spec: `a=linux,b=android@tcp:9223`. */
  TestDevices: 'FINI_E2E_TEST_DEVICES',
  /** The platforms that this job can start as virtual test devices: `linux,android`. */
  VirtualPlatforms: 'FINI_E2E_VIRTUAL_PLATFORMS',
  /** `1` keeps the run folder after a passed run. */
  Keep: 'FINI_E2E_KEEP',
} as const;
