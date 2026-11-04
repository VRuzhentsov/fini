import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { computed, ref } from "vue";
import { sortHistoryItems } from "../utils/historyGrouping";
import { parseChecklist, serializeChecklist } from "../utils/checklist";
import { localDateStr, formatDue, formatTime } from "../utils/date";
import type { EditScope } from "../utils/quest";

export type Energy = "small" | "medium" | "large";
export type Priority = "low" | "medium" | "high";

export function priorityWeight(priority: Priority): number {
  if (priority === "high") return 3;
  if (priority === "low") return 1;
  return 2;
}

/** Label for a non-active quest's status badge. Active quests have no status label. */
export function statusLabel(status: "completed" | "abandoned"): string {
  return status === "completed" ? "Completed" : "Abandoned";
}

/** Due-badge color class: overdue, due today, or upcoming. Empty when there's no due date. */
export function dueBadgeClass(due: string | null): string {
  if (!due) return "";
  const todayStr = localDateStr(new Date());
  if (due < todayStr) return "badge-error";
  if (due === todayStr) return "badge-success";
  return "badge-ghost";
}

/** "Today, 2:30 PM" / "Tomorrow" / "Jan 5, 2:30 PM" — `todayLabel`/`tomorrowLabel` are the
 * caller's translated strings, keeping this function free of an i18n dependency. */
export function smartDueLabel(
  due: string | null,
  dueTime: string | null,
  todayLabel: string,
  tomorrowLabel: string,
): string {
  if (!due) return "";
  const now = new Date();
  const todayStr = localDateStr(now);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const tomorrowStr = localDateStr(tomorrow);
  const time = dueTime ? `, ${formatTime(dueTime)}` : "";
  if (due === todayStr) return todayLabel + time;
  if (due === tomorrowStr) return tomorrowLabel + time;
  return formatDue(due) + time;
}

export interface Quest {
  id: string;
  space_id: string;
  title: string;
  description: string | null;
  status: "active" | "completed" | "abandoned";
  energy: Energy;
  priority: Priority;
  due: string | null;
  due_time: string | null;
  repeat_rule: string | null;
  completed_at: string | null;
  order_rank: number;
  focus_enter_count: number;
  created_at: string;
  updated_at: string;
  /** When true, `description` is task-list text rendered/edited as a checklist (#128). */
  is_checklist: boolean;
}

export { EditScope } from "../utils/quest";

/** Whether a quest repeats. A repeating quest is one quest; the backend
 * resolves each row of it to one occurrence, whose `due` is that day (ADR-0012). */
export function isRepeating(quest: Pick<Quest, "repeat_rule">): boolean {
  return !!quest.repeat_rule && quest.repeat_rule !== "null";
}

/** A list key that is unique per row: a repeating quest has one row per day
 * (its open day, and each closed day in History). */
export function questRowKey(quest: Pick<Quest, "id" | "repeat_rule" | "due">): string {
  return isRepeating(quest) ? `${quest.id}:${quest.due ?? ""}` : quest.id;
}

export interface ChecklistActivity {
  id: string;
  quest_id: string;
  /** "added" | "removed" | "edited" | "completed_snapshot" | "post_completion_edit" */
  kind: string;
  detail: string;
  created_at: string;
  origin_device_id: string | null;
}

export interface CreateQuestInput {
  space_id?: string;
  title: string;
  description?: string | null;
  energy?: Energy;
  priority?: Priority;
  due?: string | null;
  due_time?: string | null;
  repeat_rule?: string | null;
  order_rank?: number;
  is_checklist?: boolean;
}

export interface UpdateQuestInput {
  space_id?: string;
  title?: string;
  description?: string | null;
  status?: "active" | "completed" | "abandoned";
  energy?: Energy;
  priority?: Priority;
  due?: string | null;
  due_time?: string | null;
  repeat_rule?: string | null;
  order_rank?: number;
  /** A repeating quest only: "day" writes the edit on one occurrence; "all" (default) on the quest. */
  scope?: EditScope;
  /** A repeating quest only: the occurrence (its date) the edit is for; default its oldest open day. */
  period_key?: string;
}

/** The order of active quests: overdue first, then scheduled, then by date,
 * then `order_rank`, then Priority (high first), then the oldest. */
function activeOrder(today: string) {
  return (a: Quest, b: Quest): number =>
    Number(!!a.due && a.due < today ? 0 : 1) - Number(!!b.due && b.due < today ? 0 : 1) ||
    Number(a.due || a.repeat_rule ? 0 : 1) - Number(b.due || b.repeat_rule ? 0 : 1) ||
    Number(a.due ? 0 : 1) - Number(b.due ? 0 : 1) ||
    (a.due && b.due ? a.due.localeCompare(b.due) : 0) ||
    a.order_rank - b.order_rank ||
    priorityWeight(b.priority) - priorityWeight(a.priority) ||
    a.created_at.localeCompare(b.created_at);
}

export const useQuestStore = defineStore("quest", () => {
  const quests = ref<Quest[]>([]);
  const activeQuest = ref<Quest | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);

  /** The active quests, in their order. A repeating quest is here as its oldest open day. */
  const activeQuests = computed(() =>
    quests.value.filter((q) => q.status === "active").sort(activeOrder(localDateStr(new Date()))),
  );
  /** The completed and abandoned quests and days, newest first (History). */
  const historyQuests = computed(() =>
    sortHistoryItems(quests.value.filter((q) => q.status === "completed" || q.status === "abandoned")),
  );

  async function fetchActiveQuest() {
    activeQuest.value = await invoke<Quest | null>("get_active_focus");
  }

  async function fetchQuests() {
    loading.value = true;
    error.value = null;
    try {
      quests.value = await invoke<Quest[]>("get_quests");
      await fetchActiveQuest();
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function createQuest(input: CreateQuestInput) {
    const quest = await invoke<Quest>("create_quest", { input });
    await fetchQuests();
    return quest;
  }

  async function updateQuest(
    id: string,
    input: UpdateQuestInput,
    options?: { refresh?: boolean },
  ) {
    const quest = await invoke<Quest>("update_quest", { id, input });
    if (options?.refresh === false) {
      applyLocalQuest(quest);
    } else {
      await fetchQuests();
    }
    return quest;
  }

  /** "Complete through today" (ADR-0012 D7): closes every open occurrence of
   * a repeating quest dated today or before. */
  async function completeThroughToday(id: string) {
    const quest = await invoke<Quest>("complete_quest_through_today", { id });
    await fetchQuests();
    return quest;
  }

  async function setFocusQuest(id: string) {
    const quest = await invoke<Quest>("set_focus", { id });
    await fetchQuests();
    activeQuest.value = quest;
    return quest;
  }

  async function deleteQuest(id: string) {
    await invoke("delete_quest", { id });
    await fetchQuests();
  }

  /** Replaces `quest` wherever it's held locally (list row + active quest), without a refetch.
   * A repeating quest replaces its open row (History rows of other days stay). */
  function applyLocalQuest(quest: Quest) {
    const idx = quests.value.findIndex(
      (q) => q.id === quest.id && (!isRepeating(q) || q.status === "active"),
    );
    if (idx !== -1) quests.value[idx] = quest;
    if (activeQuest.value?.id === quest.id) activeQuest.value = quest;
  }

  // ── Checklist (issue #128) ────────────────────────────────────────────────
  // Mutations round-trip through the backend (which owns the merge-safe write); the item id
  // returned by the server response is what callers should key on afterward. `toggleChecklistItem`
  // additionally applies an optimistic local update so the checkbox feels instant.

  // A repeating quest's checklist edit goes to one day ("day") or to the
  // quest's template ("all", default), which reaches every open day (ADR-0012 D5).

  async function addChecklistItem(questId: string, text: string, scope?: EditScope) {
    const quest = await invoke<Quest>("add_checklist_item", { questId, text, scope });
    applyLocalQuest(quest);
    return quest;
  }

  /** A tick is always the day's: it has no scope (ADR-0012 D8). */
  async function toggleChecklistItem(questId: string, itemId: string, checked: boolean) {
    const current =
      quests.value.find((q) => q.id === questId && (!isRepeating(q) || q.status === "active")) ??
      (activeQuest.value?.id === questId ? activeQuest.value : null);
    if (current) {
      const items = parseChecklist(current.description).map((it) =>
        it.id === itemId ? { ...it, checked } : it,
      );
      applyLocalQuest({ ...current, description: serializeChecklist(items) });
    }
    let quest: Quest;
    try {
      quest = await invoke<Quest>("toggle_checklist_item", { questId, itemId, checked });
    } catch (e) {
      if (current) applyLocalQuest(current);
      throw e;
    }
    applyLocalQuest(quest);
    return quest;
  }

  async function editChecklistItemText(questId: string, itemId: string, text: string, scope?: EditScope) {
    const quest = await invoke<Quest>("edit_checklist_item", { questId, itemId, text, scope });
    applyLocalQuest(quest);
    return quest;
  }

  async function removeChecklistItem(questId: string, itemId: string, scope?: EditScope) {
    const quest = await invoke<Quest>("remove_checklist_item", { questId, itemId, scope });
    applyLocalQuest(quest);
    return quest;
  }

  async function reorderChecklist(questId: string, orderedItemIds: string[], scope?: EditScope) {
    const quest = await invoke<Quest>("reorder_checklist", { questId, orderedItemIds, scope });
    applyLocalQuest(quest);
    return quest;
  }

  async function fetchChecklistActivity(questId: string) {
    return invoke<ChecklistActivity[]>("get_checklist_activity", { questId });
  }

  return {
    quests,
    activeQuests,
    historyQuests,
    activeQuest,
    loading,
    error,
    fetchQuests,
    fetchActiveQuest,
    createQuest,
    updateQuest,
    completeThroughToday,
    setFocusQuest,
    deleteQuest,
    addChecklistItem,
    toggleChecklistItem,
    editChecklistItemText,
    removeChecklistItem,
    reorderChecklist,
    fetchChecklistActivity,
  };
});
