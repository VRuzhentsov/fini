<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import ActiveQuestPanel from "../components/MainView/ActiveQuestPanel.vue";
import NewQuestForm from "../components/MainView/NewQuestForm.vue";
import QuestList from "../components/QuestsView/QuestList.vue";

const store = useQuestStore();

onMounted(() => store.fetchQuests());

const backlog = computed(() => {
  const mainId = store.activeQuest?.id;
  const today = new Date().toISOString().slice(0, 10);
  return store.quests
    .filter((q) => q.status === "active" && q.id !== mainId)
    .sort((a, b) => {
      const aOverdue = a.due && a.due < today ? 1 : 0;
      const bOverdue = b.due && b.due < today ? 1 : 0;
      if (aOverdue !== bOverdue) return bOverdue - aOverdue;
      if (a.order_rank !== b.order_rank) return a.order_rank - b.order_rank;
      if (a.priority !== b.priority) return b.priority - a.priority;
      return a.created_at.localeCompare(b.created_at);
    });
});
</script>

<template>
  <div class="flex flex-col gap-8 px-4 pt-4">
    <div v-if="store.error" class="text-error text-sm">{{ store.error }}</div>

    <section>
      <ActiveQuestPanel v-if="store.activeQuest" :quest="store.activeQuest" />
      <p v-else class="text-sm opacity-40">No active quest.</p>
    </section>

    <section v-if="backlog.length">
      <QuestList :quests="backlog" />
    </section>

    <section>
      <NewQuestForm />
    </section>
  </div>
</template>
