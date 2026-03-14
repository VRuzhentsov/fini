<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import QuestList from "../components/QuestsView/QuestList.vue";
import ChatInput from "../components/ChatInput.vue";

const questStore = useQuestStore();

onMounted(() => questStore.fetchQuests());

const activeQuests = computed(() =>
  questStore.quests.filter((q) => q.status === "active")
);

async function onSubmit(text: string) {
  await questStore.createQuest({ title: text });
}
</script>

<template>
  <div class="flex flex-col gap-4 px-4 pt-4 pb-24">
    <div v-if="questStore.error" class="text-error text-sm">{{ questStore.error }}</div>

    <p v-if="!activeQuests.length" class="text-sm opacity-40">No active quests.</p>
    <QuestList v-else :quests="activeQuests" />
  </div>

  <ChatInput @submit="onSubmit" />
</template>
