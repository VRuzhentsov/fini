/**
 * Formats an ISO timestamp as an elapsed phrase -- "just now", "2 min ago",
 * "3 h ago", "5 days ago".
 *
 * Used where the exact moment is noise and the age is the point: "last
 * change reached Pixel 8 2 min ago" answers "is it keeping up?", which
 * "Today, 14:32" does not. Prefer `formatTimestamp` wherever the reader
 * might want to line the moment up against something else.
 *
 * A future timestamp reads as "just now" rather than negative: clocks on
 * two devices are not the same clock, and a peer a few seconds ahead should
 * not produce "in 4 seconds".
 */
export function relativeTime(raw: string, now: number = Date.now()): string {
  const then = new Date(raw).getTime();
  if (Number.isNaN(then)) return "";

  const seconds = Math.round((now - then) / 1000);
  if (seconds < 45) return "just now";

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;

  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;

  const days = Math.round(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

/** Formats an ISO timestamp as "Today, HH:MM", "Yesterday, HH:MM", or "Mon D, HH:MM". */
export function formatTimestamp(raw: string): string {
  const date = new Date(raw);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const time = date.toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", hour12: false });
  if (date.toDateString() === today.toDateString()) return `Today, ${time}`;
  if (date.toDateString() === yesterday.toDateString()) return `Yesterday, ${time}`;
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" }) + `, ${time}`;
}
