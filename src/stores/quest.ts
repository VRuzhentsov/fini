import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";
import { parseChecklist, serializeChecklist } from "../utils/checklistMarkdown";

export type Energy = "low" | "medium" | "high";

export interface Quest {
  id: string;
  space_id: string;
  title: string;
  description: string | null;
  status: "active" | "completed" | "abandoned";
  energy: Energy;
  priority: number;
  pinned: boolean;
  due: string | null;
  due_time: string | null;
  repeat_rule: string | null;
  completed_at: string | null;
  order_rank: number;
  focus_enter_count: number;
  created_at: string;
  updated_at: string;
  series_id: string | null;
  period_key: string | null;
  /** When true, `description` is task-list markdown rendered/edited as a checklist (#128). */
  is_checklist: boolean;
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
  priority?: number;
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
  priority?: number;
  pinned?: boolean;
  due?: string | null;
  due_time?: string | null;
  repeat_rule?: string | null;
  order_rank?: number;
}

export const useQuestStore = defineStore("quest", () => {
  const quests = ref<Quest[]>([]);
  const activeQuest = ref<Quest | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);

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
      const idx = quests.value.findIndex((q) => q.id === id);
      if (idx !== -1) quests.value[idx] = quest;
      if (activeQuest.value?.id === id) activeQuest.value = quest;
    } else {
      await fetchQuests();
    }
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

  /** Replaces `quest` wherever it's held locally (list row + active quest), without a refetch. */
  function applyLocalQuest(quest: Quest) {
    const idx = quests.value.findIndex((q) => q.id === quest.id);
    if (idx !== -1) quests.value[idx] = quest;
    if (activeQuest.value?.id === quest.id) activeQuest.value = quest;
  }

  // ── Checklist (issue #128) ────────────────────────────────────────────────
  // Mutations round-trip through the backend (which owns the merge-safe write); the item id
  // returned by the server response is what callers should key on afterward. `toggleChecklistItem`
  // additionally applies an optimistic local update so the checkbox feels instant.

  async function addChecklistItem(questId: string, text: string) {
    const quest = await invoke<Quest>("add_checklist_item", { questId, text });
    applyLocalQuest(quest);
    return quest;
  }

  async function toggleChecklistItem(questId: string, itemId: string, checked: boolean) {
    const current =
      quests.value.find((q) => q.id === questId) ??
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

  async function editChecklistItemText(questId: string, itemId: string, text: string) {
    const quest = await invoke<Quest>("edit_checklist_item", { questId, itemId, text });
    applyLocalQuest(quest);
    return quest;
  }

  async function removeChecklistItem(questId: string, itemId: string) {
    const quest = await invoke<Quest>("remove_checklist_item", { questId, itemId });
    applyLocalQuest(quest);
    return quest;
  }

  async function reorderChecklist(questId: string, orderedItemIds: string[]) {
    const quest = await invoke<Quest>("reorder_checklist", { questId, orderedItemIds });
    applyLocalQuest(quest);
    return quest;
  }

  /** `scope` is `"this"` or `"future"` — see issue #128's recurrence edit-scope contract. */
  async function updateSeriesChecklist(
    seriesId: string,
    currentOccurrenceId: string,
    checklistMd: string,
    scope: "this" | "future",
  ) {
    const quest = await invoke<Quest>("update_series_checklist", {
      seriesId,
      currentOccurrenceId,
      checklistMd,
      scope,
    });
    applyLocalQuest(quest);
    return quest;
  }

  async function fetchChecklistActivity(questId: string) {
    return invoke<ChecklistActivity[]>("get_checklist_activity", { questId });
  }

  return {
    quests,
    activeQuest,
    loading,
    error,
    fetchQuests,
    fetchActiveQuest,
    createQuest,
    updateQuest,
    setFocusQuest,
    deleteQuest,
    addChecklistItem,
    toggleChecklistItem,
    editChecklistItemText,
    removeChecklistItem,
    reorderChecklist,
    updateSeriesChecklist,
    fetchChecklistActivity,
  };
});
