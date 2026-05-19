<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import QuestList from "../components/QuestsView/QuestList.vue";
import { sortHistoryItems } from "../utils/historyGrouping";

const store = useQuestStore();
const history = computed(() =>
  store.quests.filter((q) => q.status === "completed" || q.status === "abandoned")
);
const sorted = computed(() => sortHistoryItems(history.value));

onMounted(() => store.fetchQuests());
</script>

<template>
  <div class="flex flex-col gap-3">
    <div v-if="store.error" class="text-error text-sm">{{ store.error }}</div>

    <p v-if="!sorted.length" class="text-sm opacity-40">No completed or abandoned quests yet.</p>
    <QuestList v-else :quests="sorted" />
  </div>
</template>
