import type { ChannelStatusCode } from "../stores/device";

// Mirrors the backend's ChannelStatusCode enum (device_connection::
// channel, ADR-0003 revision). This is the ONLY place English text is
// attached to a code -- the `code` tag itself is the stable key, chosen
// specifically so a future locale table can key translations off it
// without touching the backend. Swap this file for a locale-aware lookup
// when i18n lands; callers should keep going through `channelStatusText`
// rather than switching on `code.code` themselves.
//
// The wording is the device-page redesign's central requirement: a channel
// row says *why* it isn't connected in words the person can act on, never
// a status code and never a bare coloured dot. Two rules follow from that:
//
//  - Name the device. "Isn't nearby" is ambiguous about which of the two
//    devices is the problem; "Pixel 8 isn't nearby" is not.
//  - Say which machine is at fault. The difference between "this computer"
//    and the peer is the difference between something the person can fix
//    where they are standing and something they cannot.
export function channelStatusText(
  code: ChannelStatusCode,
  peerName?: string,
): string {
  // Falls back to a pronoun rather than an empty string: the sentence has
  // to stay grammatical when a caller has no name to hand.
  const peer = peerName?.trim() || "That device";

  switch (code.code) {
    case "network_unavailable":
      return `${peer} isn't on this network`;
    case "network_disabled":
      return "Network is switched off for this device";
    case "bluetooth_not_supported":
      return "This computer can't use Bluetooth";
    case "bluetooth_disabled":
      return "Bluetooth is switched off for this device";
    case "bluetooth_adapter_off":
      // Carries the promise as well as the condition: this is a state the
      // person sits in rather than passes through, so the sentence has to
      // answer "and then what" on its own, whenever they ask for it.
      return "Bluetooth is off on this computer — the channel starts by itself once you turn it on";
    case "bluetooth_no_address":
      return `No Bluetooth address for ${peer} yet`;
    case "bluetooth_peer_not_nearby":
      return `${peer} isn't nearby`;
    case "bluetooth_dial_exhausted":
      return "Couldn't connect";
    case "connecting":
      return "Connecting…";
    case "awaiting_first_ack":
      return "Connecting…";
    case "ping_missed":
      return "Not answering";
  }
}

// What a channel row reads as, derived from the backend's row state plus
// the pair's own switch. Mirrors the redesign's own vocabulary rather than
// the backend's, because the two answer different questions: the backend
// says what is true of the link, this says what the person is looking at.
export type ChannelRowState =
  // The switch is off. Nothing is happening because the user said so.
  | "off"
  // The switch is on, and this machine's own radio is what's missing. Gray
  // dot, and the switch sits in the on position with a gray track.
  | "waiting"
  // The switch is on and the radio is fine -- the other device is the one
  // that can't be reached.
  | "down"
  // Dialling, or dialled but not yet proven.
  | "connecting"
  // Proven live, but the proof has since lapsed.
  | "fading"
  // Proven live right now.
  | "connected";

export function channelRowState(
  state:
    | { state: "unconfigured"; code: ChannelStatusCode }
    | { state: "configured"; code: ChannelStatusCode | null },
  enabled: boolean,
): ChannelRowState {
  if (!enabled) return "off";

  if (state.state === "unconfigured") {
    switch (state.code.code) {
      // The two codes that mean "this machine", not "the peer". Both read
      // as waiting rather than down, because the switch is on and the thing
      // that would make it work is here rather than somewhere else.
      case "bluetooth_adapter_off":
      case "bluetooth_not_supported":
        return "waiting";
      default:
        return "down";
    }
  }

  if (state.code === null) return "connected";
  return state.code.code === "ping_missed" ? "fading" : "connecting";
}

// The one-line label beside the dot. Deliberately short and never
// punctuated -- the reason lives in the info button next to it, and
// duplicating it here would make a two-line row out of a one-line fact.
export function channelRowLabel(row: ChannelRowState): string {
  switch (row) {
    case "off":
      return "Off";
    case "waiting":
      return "On, waiting";
    case "down":
      return "Not connected";
    case "connecting":
      return "Connecting…";
    case "fading":
      return "Connected, not answering";
    case "connected":
      return "Connected";
  }
}
