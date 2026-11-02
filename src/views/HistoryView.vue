<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import QuestList from "../components/QuestsView/QuestList.vue";

const store = useQuestStore();
const sorted = computed(() => store.historyQuests);

onMounted(() => store.fetchQuests());
</script>

<template>
  <div class="flex flex-col gap-3">
    <div v-if="store.error" class="text-error text-sm">{{ store.error }}</div>

    <p v-if="!sorted.length" class="text-sm opacity-40">No completed or abandoned quests yet.</p>
    <QuestList v-else :quests="sorted" />
  </div>
</template>
