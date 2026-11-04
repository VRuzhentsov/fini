import { isRepeating, type Quest } from "../stores/quest";

export function historyQuestTime(quest: Quest): string {
  return quest.completed_at ?? quest.updated_at;
}

/** History rows, newest first. A repeating quest has one row per closed day;
 * History shows its newest one (ADR-0012). */
export function sortHistoryItems(history: Quest[]): Quest[] {
  const latestByQuest = new Map<string, Quest>();
  const rows: Quest[] = [];

  for (const quest of history) {
    if (!isRepeating(quest)) {
      rows.push(quest);
      continue;
    }
    const existing = latestByQuest.get(quest.id);
    if (!existing || historyQuestTime(quest) > historyQuestTime(existing)) {
      latestByQuest.set(quest.id, quest);
    }
  }

  rows.push(...latestByQuest.values());
  rows.sort((a, b) => {
    const aTime = historyQuestTime(a);
    const bTime = historyQuestTime(b);
    if (aTime !== bTime) return bTime.localeCompare(aTime);
    return a.id.localeCompare(b.id);
  });

  return rows;
}
