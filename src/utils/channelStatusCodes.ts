import type { ChannelRowState, ChannelStatusCode } from "../stores/device";

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
  channelName?: string,
): string {
  // Falls back to a pronoun rather than an empty string: the sentence has
  // to stay grammatical when a caller has no name to hand.
  const peer = peerName?.trim() || "That device";

  switch (code) {
    case "peer_not_on_network":
      return `${peer} isn't on this network`;
    case "bluetooth_not_supported":
      return "This computer can't use Bluetooth";
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
    case "awaiting_first_ack":
      return "Connecting…";
    case "not_answering":
      return "Not answering";
    // One sentence for both channels, with the name substituted in --
    // which is why there is no longer a code per channel for it.
    case "disabled":
      return `${channelName ?? "This channel"} is switched off for this device`;
  }
}

// The row's category comes from the backend now (`DeviceChannelStatus.status`).
// `channelRowState` used to compute it here, which meant switching on
// `bluetooth_adapter_off` and `bluetooth_not_supported` to decide "waiting"
// versus "down" -- one channel's failure modes hard-coded into the one
// function that is supposed to hold for every channel. A channel
// categorises its own reasons now; see `ChannelReason::category`.

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
