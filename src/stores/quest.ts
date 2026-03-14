import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

export type Energy = "low" | "medium" | "high";

export interface Quest {
  id: number;
  space_id: number | null;
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
  created_at: string;
  updated_at: string;
}

export interface CreateQuestInput {
  space_id?: number | null;
  title: string;
  description?: string | null;
  energy?: Energy;
  priority?: number;
  due?: string | null;
  due_time?: string | null;
  repeat_rule?: string | null;
}

export interface UpdateQuestInput {
  space_id?: number | null;
  title?: string;
  description?: string | null;
  status?: "active" | "completed" | "abandoned";
  energy?: Energy;
  priority?: number;
  pinned?: boolean;
  due?: string | null;
  due_time?: string | null;
  repeat_rule?: string | null;
}

export const useQuestStore = defineStore("quest", () => {
  const quests = ref<Quest[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function fetchQuests() {
    loading.value = true;
    error.value = null;
    try {
      quests.value = await invoke<Quest[]>("get_quests");
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function createQuest(input: CreateQuestInput) {
    const quest = await invoke<Quest>("create_quest", { input });
    quests.value.unshift(quest);
    return quest;
  }

  async function updateQuest(id: number, input: UpdateQuestInput) {
    const quest = await invoke<Quest>("update_quest", { id, input });
    const idx = quests.value.findIndex((q) => q.id === id);
    if (idx !== -1) quests.value[idx] = quest;
    return quest;
  }

  async function deleteQuest(id: number) {
    await invoke("delete_quest", { id });
    quests.value = quests.value.filter((q) => q.id !== id);
  }

  return { quests, loading, error, fetchQuests, createQuest, updateQuest, deleteQuest };
});
