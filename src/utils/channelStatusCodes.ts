import { ChannelKind, ChannelProblem } from "./channel";

// Mirrors the backend's `ChannelProblem` (ADR-0008 D19). This is the ONLY
// place English text is attached to a problem -- the key itself is stable,
// so a future locale table can key translations off it without touching the
// backend.
//
// Say which machine is at fault: a problem is always on *this* device (the
// peer being away is grey, not a problem), and saying so is what tells the
// person it is something they can fix where they are standing.
export function channelProblemText(problem: ChannelProblem): string {
  switch (problem) {
    case ChannelProblem.BluetoothNotSupported:
      return "This device can't use Bluetooth";
    case ChannelProblem.BluetoothUnavailable:
      return "Bluetooth is unavailable on this device — the channel resumes by itself once it works again";
    case ChannelProblem.NetworkUnavailable:
      return "This device can't reach the local network";
  }
}

export function channelName(kind: ChannelKind): string {
  return kind === ChannelKind.Network ? "Network" : "Bluetooth";
}
