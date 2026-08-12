import { formatMacAddress, macAddressHexDigits } from "../../utils/macAddress";

describe("macAddressHexDigits", () => {
  it("uppercases and keeps only hex characters", () => {
    expect(macAddressHexDigits("aabbccddeeff")).toBe("AABBCCDDEEFF");
  });

  it("strips separators from an already-formatted paste", () => {
    expect(macAddressHexDigits("AA:BB:CC:DD:EE:FF")).toBe("AABBCCDDEEFF");
    expect(macAddressHexDigits("aa-bb-cc-dd-ee-ff")).toBe("AABBCCDDEEFF");
  });

  it("truncates at 12 hex digits (6 octets)", () => {
    expect(macAddressHexDigits("AABBCCDDEEFFGGHH")).toBe("AABBCCDDEEFF");
    expect(macAddressHexDigits("aabbccddeeff1122")).toBe("AABBCCDDEEFF");
  });

  it("drops non-hex letters and punctuation entirely, not just separators", () => {
    expect(macAddressHexDigits("zzAAbbZZccQQ")).toBe("AABBCC");
  });

  it("handles empty and partial input", () => {
    expect(macAddressHexDigits("")).toBe("");
    expect(macAddressHexDigits("a")).toBe("A");
  });
});

describe("formatMacAddress", () => {
  it("groups a full address into colon-separated octets", () => {
    expect(formatMacAddress("AABBCCDDEEFF")).toBe("AA:BB:CC:DD:EE:FF");
  });

  it("formats a partial, in-progress value without a trailing colon", () => {
    expect(formatMacAddress("A")).toBe("A");
    expect(formatMacAddress("AA")).toBe("AA");
    expect(formatMacAddress("AAB")).toBe("AA:B");
    expect(formatMacAddress("AABB")).toBe("AA:BB");
  });

  it("formats an empty value as an empty string", () => {
    expect(formatMacAddress("")).toBe("");
  });

  it("round-trips through macAddressHexDigits for any pasted format", () => {
    for (const pasted of ["AA:BB:CC:DD:EE:FF", "aabbccddeeff", "aa-bb-cc-dd-ee-ff"]) {
      expect(formatMacAddress(macAddressHexDigits(pasted))).toBe("AA:BB:CC:DD:EE:FF");
    }
  });
});
