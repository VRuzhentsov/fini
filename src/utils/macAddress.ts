/** Uppercase hex digits only, capped at 12 (6 octets) -- the raw form a MAC address input mask stores between keystrokes. */
export function macAddressHexDigits(value: string): string {
  return value.toUpperCase().replace(/[^0-9A-F]/g, "").slice(0, 12);
}

/** Groups raw hex digits into "AA:BB:CC:DD:EE:FF" form, colon-separated as they arrive rather than only once 12 digits are typed. */
export function formatMacAddress(hexDigits: string): string {
  return hexDigits.match(/.{1,2}/g)?.join(":") ?? "";
}
