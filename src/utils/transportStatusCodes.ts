import type { TransportStatusCode } from "../stores/device";

// Mirrors the backend's TransportStatusCode enum (device_connection::
// transport, ADR-0003 revision). This is the ONLY place English text is
// attached to a code -- the `code` tag itself is the stable key, chosen
// specifically so a future locale table can key translations off it
// without touching the backend. Swap this file for a locale-aware lookup
// when i18n lands; callers should keep going through `transportStatusText`
// rather than switching on `code.code` themselves.
export function transportStatusText(code: TransportStatusCode): string {
  switch (code.code) {
    case "network_unavailable":
      return "No network presence detected for this peer.";
    case "bluetooth_not_supported":
      return "Bluetooth transport isn't available on this platform.";
    case "bluetooth_disabled":
      return "Bluetooth is disabled for this pair.";
    case "bluetooth_no_address":
      return "No Bluetooth address stored yet -- pair over OS Bluetooth or use Find via Bluetooth.";
    case "bluetooth_not_os_paired":
      return "OS Bluetooth pairing is required.";
    case "awaiting_first_ack":
      return "Connected -- waiting for the first ping/ack exchange to confirm the link.";
    case "ping_missed":
      return `Connected, but ${code.count} consecutive ping${code.count === 1 ? "" : "s"} went unanswered.`;
  }
}
