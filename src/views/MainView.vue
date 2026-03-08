<script setup lang="ts">
import { onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import ActiveQuestPanel from "../components/ActiveQuestPanel.vue";
import NewQuestForm from "../components/NewQuestForm.vue";
import QuestList from "../components/QuestList.vue";

const store = useQuestStore();

onMounted(async () => {
  await Promise.all([store.fetchActiveQuest(), store.fetchQuests()]);
});
</script>

<template>
  <div class="main-view">
    <div v-if="store.error" class="error">{{ store.error }}</div>

    <section class="active-section">
      <ActiveQuestPanel v-if="store.activeQuest" :quest="store.activeQuest" />
      <div v-else class="no-quest">No active quest.</div>
    </section>

    <section class="input-section">
      <NewQuestForm />
    </section>

    <section class="history-section" v-if="store.quests.length">
      <h3>All Quests</h3>
      <QuestList :quests="store.quests" />
    </section>
  </div>
</template>

<style scoped>
.main-view {
  padding: 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 2rem;
}

.error {
  color: red;
  font-size: 0.875rem;
}

.no-quest {
  opacity: 0.4;
  font-size: 0.875rem;
}

.history-section h3 {
  margin-bottom: 0.75rem;
  font-size: 0.875rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  opacity: 0.5;
}
</style>
