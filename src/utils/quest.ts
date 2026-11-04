// Quest value sets, with no runtime imports so specs and e2e helpers can
// import them (fini-code-style, "No magic strings").

/** The status of a quest, or of one day of a repeating quest. */
export const QuestStatus = {
  Active: "active",
  Completed: "completed",
  Abandoned: "abandoned",
} as const;
export type QuestStatus = (typeof QuestStatus)[keyof typeof QuestStatus];

/** The edit scope of a repeating quest (ADR-0012 D8): one day, or the quest. */
export const EditScope = {
  Day: "day",
  All: "all",
} as const;
export type EditScope = (typeof EditScope)[keyof typeof EditScope];
