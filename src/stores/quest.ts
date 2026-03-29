import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

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
  set_focus_at: string | null;
  reminder_triggered_at: string | null;
  order_rank: number;
  created_at: string;
  updated_at: string;
  series_id: string | null;
  period_key: string | null;
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
  };
});
