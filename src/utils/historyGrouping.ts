import type { Quest } from "../stores/quest";

export function historyQuestTime(quest: Quest): string {
  return quest.completed_at ?? quest.updated_at;
}

function compareDescByHistoryTime(a: Quest, b: Quest): number {
  const aTime = historyQuestTime(a);
  const bTime = historyQuestTime(b);
  if (aTime !== bTime) return bTime.localeCompare(aTime);
  return a.id.localeCompare(b.id);
}

export function groupHistoryItems(history: Quest[]): {
  rows: Quest[];
  groupChildrenById: Record<string, Quest[]>;
} {
  const bySeries = new Map<string, Quest[]>();
  for (const quest of history) {
    if (!quest.series_id) continue;
    const bucket = bySeries.get(quest.series_id) ?? [];
    bucket.push(quest);
    bySeries.set(quest.series_id, bucket);
  }

  const groupedSeriesIds = new Set(
    [...bySeries.entries()]
      .filter(([, children]) => children.length >= 2)
      .map(([id]) => id),
  );

  const rows: Quest[] = [];
  const groupChildrenById: Record<string, Quest[]> = {};
  const emittedSeriesIds = new Set<string>();

  for (const quest of history) {
    const seriesId = quest.series_id;
    if (!seriesId || !groupedSeriesIds.has(seriesId)) {
      rows.push(quest);
      continue;
    }
    if (emittedSeriesIds.has(seriesId)) continue;
    emittedSeriesIds.add(seriesId);

    const children = [...(bySeries.get(seriesId) ?? [])].sort(compareDescByHistoryTime);
    const representative = children[0];
    groupChildrenById[representative.id] = children;
    rows.push(representative);
  }

  rows.sort((a, b) => {
    const aTime = historyQuestTime(a);
    const bTime = historyQuestTime(b);
    if (aTime !== bTime) return bTime.localeCompare(aTime);
    // Tie-break by id: deterministic but not semantically meaningful; adequate for identical timestamps.
    return a.id.localeCompare(b.id);
  });

  return { rows, groupChildrenById };
}
