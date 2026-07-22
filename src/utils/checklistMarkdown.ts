export interface ChecklistItem {
  id: string;
  text: string;
  checked: boolean;
}

/** New checklist item id. Falls back when `crypto.randomUUID` is unavailable (e.g. jsdom in
 * tests) — mirrors the same guard already used in src/stores/device.ts. */
export function newChecklistItemId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `${Date.now().toString(16)}-${Math.random().toString(16).slice(2, 10)}`;
}

/**
 * Parses task-list lines (`- [ ] text` / `- [x] text`) with an optional trailing hidden id token
 * (`<!--k=id-->`) out of a quest's `description`. Mirrors src-tauri/src/services/checklist_md.rs
 * `parse` — kept in sync manually since the two run in different languages.
 */
export function parseChecklist(src: string | null | undefined): ChecklistItem[] {
  if (!src) return [];
  const items: ChecklistItem[] = [];
  for (const rawLine of src.split("\n")) {
    const line = rawLine.trim();
    let checked: boolean;
    let rest: string;
    if (line.startsWith("- [ ] ")) {
      checked = false;
      rest = line.slice(6);
    } else if (line.startsWith("- [x] ") || line.startsWith("- [X] ")) {
      checked = true;
      rest = line.slice(6);
    } else {
      continue;
    }

    const idMatch = rest.match(/<!--k=([^>]*)-->$/);
    if (idMatch) {
      items.push({
        id: idMatch[1],
        text: rest.slice(0, idMatch.index).trimEnd(),
        checked,
      });
    } else {
      items.push({ id: newChecklistItemId(), text: rest, checked });
    }
  }
  return items;
}

export function serializeChecklist(items: ChecklistItem[]): string {
  return items
    .map((it) => `- [${it.checked ? "x" : " "}] ${it.text} <!--k=${it.id}-->`)
    .join("\n");
}

/** `[done, total]` counts, for list/editor badges. */
export function checklistCounts(src: string | null | undefined): [number, number] {
  const items = parseChecklist(src);
  return [items.filter((it) => it.checked).length, items.length];
}
