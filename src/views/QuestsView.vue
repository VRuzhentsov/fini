<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import QuestList from "../components/QuestsView/QuestList.vue";
import NewQuestForm from "../components/FocusView/NewQuestForm.vue";

const questStore = useQuestStore();

onMounted(() => questStore.fetchQuests());

const activeQuests = computed(() => questStore.activeQuests);
</script>

<template>
  <div class="flex flex-col gap-3 pb-24">
    <div v-if="questStore.error" class="text-error text-sm">{{ questStore.error }}</div>

    <p v-if="!activeQuests.length" class="text-sm opacity-40">No active quests.</p>
    <QuestList v-else :quests="activeQuests" />
  </div>

  <NewQuestForm />
</template>
