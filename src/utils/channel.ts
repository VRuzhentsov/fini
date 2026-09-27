// The channel value sets (ADR-0008), named once. Everything else -- stores,
// components, specs, e2e helpers -- goes through these, never the literals.
// The values are the backend's serde wire format. No runtime imports here,
// so e2e helpers can import this file too.

export const ChannelKind = {
  Network: "network",
  Bluetooth: "bluetooth",
} as const;
export type ChannelKind = (typeof ChannelKind)[keyof typeof ChannelKind];

// D15: the stored state, per (pair, channel).
export const ChannelState = {
  None: "none",
  Off: "off",
  On: "on",
} as const;
export type ChannelState = (typeof ChannelState)[keyof typeof ChannelState];

// D19: the row's colour, derived by the backend and only drawn here.
export const ChannelColor = {
  Green: "green",
  Grey: "grey",
  Orange: "orange",
  Off: "off",
  None: "none",
} as const;
export type ChannelColor = (typeof ChannelColor)[keyof typeof ChannelColor];

// What the ⓘ popup on an orange row explains; see `channelStatusCodes.ts`
// for the English text.
export const ChannelProblem = {
  BluetoothNotSupported: "bluetooth_not_supported",
  BluetoothUnavailable: "bluetooth_unavailable",
  NetworkUnavailable: "network_unavailable",
} as const;
export type ChannelProblem = (typeof ChannelProblem)[keyof typeof ChannelProblem];
