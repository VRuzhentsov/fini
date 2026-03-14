<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import ActiveQuestPanel from "../components/MainView/ActiveQuestPanel.vue";
import NewQuestForm from "../components/MainView/NewQuestForm.vue";

const store = useQuestStore();
const activeQuest = computed(() => store.quests.find((q) => q.status === "active") ?? null);

onMounted(() => store.fetchQuests());
</script>

<template>
  <div class="main-view">
    <div v-if="store.error" class="error">{{ store.error }}</div>

    <section class="active-section">
      <ActiveQuestPanel v-if="activeQuest" :quest="activeQuest" />
      <div v-else class="no-quest">No active quest.</div>
    </section>

    <section class="input-section">
      <NewQuestForm />
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

</style>
