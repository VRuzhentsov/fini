// The two parts of a test device (ADR-0013 D4): its platform, and its source.
// No runtime imports, so specs, the config and scripts can import it.

/** The platform of a test device. */
export const TestDevicePlatform = {
  Linux: 'linux',
  Android: 'android',
} as const;
export type TestDevicePlatform = (typeof TestDevicePlatform)[keyof typeof TestDevicePlatform];

/** Where a test device comes from. */
export const TestDeviceSource = {
  /** The harness starts it: a fini-app process on Linux, an emulator on Android. */
  Virtual: 'virtual',
  /** It runs already; the harness connects to its TCP port. */
  Physical: 'physical',
} as const;
export type TestDeviceSource = (typeof TestDeviceSource)[keyof typeof TestDeviceSource];
