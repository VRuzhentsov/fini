import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

export interface Quest {
  id: number;
  title: string;
  status: "active" | "completed" | "abandoned";
  energy_required: number | null;
  created_at: string;
  updated_at: string;
}

export interface QuestStep {
  id: number;
  quest_id: number;
  body: string;
  done: boolean;
  step_order: number;
}

export interface QuestWithSteps extends Quest {
  steps: QuestStep[];
}

export const useQuestStore = defineStore("quest", () => {
  const quests = ref<Quest[]>([]);
  const activeQuest = ref<QuestWithSteps | null>(null);
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

  async function fetchActiveQuest() {
    loading.value = true;
    error.value = null;
    try {
      activeQuest.value = await invoke<QuestWithSteps | null>("get_active_quest");
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function createQuest(title: string, energyRequired?: number) {
    error.value = null;
    const quest = await invoke<Quest>("create_quest", {
      title,
      energy_required: energyRequired ?? null,
    });
    quests.value.unshift(quest);
    return quest;
  }

  async function updateQuestStatus(id: number, status: Quest["status"]) {
    error.value = null;
    await invoke("update_quest_status", { id, status });
    const q = quests.value.find((q) => q.id === id);
    if (q) q.status = status;
    if (activeQuest.value?.id === id) {
      if (status !== "active") activeQuest.value = null;
      else activeQuest.value.status = status;
    }
  }

  async function deleteQuest(id: number) {
    error.value = null;
    await invoke("delete_quest", { id });
    quests.value = quests.value.filter((q) => q.id !== id);
    if (activeQuest.value?.id === id) activeQuest.value = null;
  }

  async function createStep(questId: number, body: string, stepOrder: number) {
    error.value = null;
    const step = await invoke<QuestStep>("create_step", {
      quest_id: questId,
      body,
      step_order: stepOrder,
    });
    if (activeQuest.value?.id === questId) {
      activeQuest.value.steps.push(step);
    }
    return step;
  }

  async function updateStepDone(id: number, done: boolean) {
    error.value = null;
    await invoke("update_step_done", { id, done });
    if (activeQuest.value) {
      const step = activeQuest.value.steps.find((s) => s.id === id);
      if (step) step.done = done;
    }
  }

  return {
    quests,
    activeQuest,
    loading,
    error,
    fetchQuests,
    fetchActiveQuest,
    createQuest,
    updateQuestStatus,
    deleteQuest,
    createStep,
    updateStepDone,
  };
});
