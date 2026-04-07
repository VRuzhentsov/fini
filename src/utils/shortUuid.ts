export type ShortUuidDirection = "start" | "end";

export function shortUuid(
  value: string,
  visibleChars = 8,
  direction: ShortUuidDirection = "start",
): string {
  if (visibleChars <= 0) return "";
  if (value.length <= visibleChars) return value;
  if (direction === "end") return value.slice(-visibleChars);
  return value.slice(0, visibleChars);
}
