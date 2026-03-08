/// Breaks a voice transcript into quest steps.
///
/// Current implementation: rule-based heuristic parser (works offline, no model needed).
/// Replace `parseSteps` with a MediaPipe LLM call when that integration is ready.

export function useStepBreakdown() {
  function parseSteps(transcript: string): string[] {
    const text = transcript.trim();
    if (!text) return [];

    // Try numbered list: "1. foo 2. bar" or "1) foo 2) bar"
    const numbered = text.match(/\d+[\.\)]\s+[^0-9]+?(?=\d+[\.\)]|$)/g);
    if (numbered && numbered.length > 1) {
      return numbered
        .map((s) => s.replace(/^\d+[\.\)]\s+/, "").trim())
        .filter(Boolean);
    }

    // Try bullet / dash list
    const bulleted = text.match(/[-•*]\s+.+/g);
    if (bulleted && bulleted.length > 1) {
      return bulleted
        .map((s) => s.replace(/^[-•*]\s+/, "").trim())
        .filter(Boolean);
    }

    // Try "then", "and then", "after that" as step connectors
    const thenSplit = text
      .split(/\b(?:and\s+)?then\b|\bafter\s+that\b|\bnext\b/i)
      .map((s) => s.trim())
      .filter((s) => s.length > 2);
    if (thenSplit.length > 1) return thenSplit;

    // Try comma-separated items (only if ≥3 items, to avoid splitting normal sentences)
    const commaSplit = text.split(/,\s+/).map((s) => s.trim()).filter(Boolean);
    if (commaSplit.length >= 3) return commaSplit;

    // Fallback: treat the whole transcript as one step
    return [text];
  }

  return { parseSteps };
}
