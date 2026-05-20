import type { Quest } from "../stores/quest";

export function historyQuestTime(quest: Quest): string {
  return quest.completed_at ?? quest.updated_at;
}

export function sortHistoryItems(history: Quest[]): Quest[] {
  const latestBySeries = new Map<string, Quest>();
  const rows: Quest[] = [];

  for (const quest of history) {
    if (!quest.series_id) {
      rows.push(quest);
      continue;
    }
    const existing = latestBySeries.get(quest.series_id);
    if (!existing || historyQuestTime(quest) > historyQuestTime(existing)) {
      latestBySeries.set(quest.series_id, quest);
    }
  }

  rows.push(...latestBySeries.values());
  rows.sort((a, b) => {
    const aTime = historyQuestTime(a);
    const bTime = historyQuestTime(b);
    if (aTime !== bTime) return bTime.localeCompare(aTime);
    return a.id.localeCompare(b.id);
  });

  return rows;
}
