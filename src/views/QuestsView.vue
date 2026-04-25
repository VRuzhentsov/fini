<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore, type Quest } from "../stores/quest";
import QuestList from "../components/QuestsView/QuestList.vue";
import ChatInput from "../components/ChatInput.vue";

const questStore = useQuestStore();

onMounted(() => questStore.fetchQuests());

function localDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function sortActiveQuests(a: Quest, b: Quest): number {
  const today = localDateStr(new Date());
  const aOverdue = a.due && a.due < today ? 1 : 0;
  const bOverdue = b.due && b.due < today ? 1 : 0;
  if (aOverdue !== bOverdue) return bOverdue - aOverdue;
  const aScheduled = a.due || a.repeat_rule ? 1 : 0;
  const bScheduled = b.due || b.repeat_rule ? 1 : 0;
  if (aScheduled !== bScheduled) return bScheduled - aScheduled;
  const aHasDue = a.due ? 1 : 0;
  const bHasDue = b.due ? 1 : 0;
  if (aHasDue !== bHasDue) return bHasDue - aHasDue;
  if (a.due && b.due && a.due !== b.due) return a.due.localeCompare(b.due);
  if (a.order_rank !== b.order_rank) return a.order_rank - b.order_rank;
  if (a.priority !== b.priority) return b.priority - a.priority;
  return a.created_at.localeCompare(b.created_at);
}

const activeQuests = computed(() =>
  questStore.quests.filter((q) => q.status === "active").sort(sortActiveQuests)
);

async function onSubmit(text: string) {
  await questStore.createQuest({ title: text });
}
</script>

<template>
  <div class="flex flex-col gap-3 pb-24">
    <div v-if="questStore.error" class="text-error text-sm">{{ questStore.error }}</div>

    <p v-if="!activeQuests.length" class="text-sm opacity-40">No active quests.</p>
    <QuestList v-else :quests="activeQuests" />
  </div>

  <ChatInput @submit="onSubmit" />
</template>
