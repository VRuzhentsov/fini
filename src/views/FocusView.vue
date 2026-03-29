<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import { useSpaceStore } from "../stores/space";
import ActiveQuestPanel from "../components/FocusView/ActiveQuestPanel.vue";
import NewQuestForm from "../components/FocusView/NewQuestForm.vue";
import QuestList from "../components/QuestsView/QuestList.vue";

const store = useQuestStore();
const spaceStore = useSpaceStore();

onMounted(() => store.fetchQuests());

const filteredQuests = computed(() => {
  const sid = spaceStore.selectedSpaceId;
  if (!sid) return store.quests;
  return store.quests.filter((q) => q.space_id === sid);
});

function localDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

const backlog = computed(() => {
  const mainId = store.activeQuest?.id;
  const today = localDateStr(new Date());
  return filteredQuests.value
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

const activeQuest = computed(() => {
  const sid = spaceStore.selectedSpaceId;
  if (!sid || !store.activeQuest) return store.activeQuest;
  return store.activeQuest.space_id === sid ? store.activeQuest : null;
});
</script>

<template>
  <div class="flex flex-col gap-1">
    <div v-if="store.error" class="text-error text-sm">{{ store.error }}</div>

    <section>
      <ActiveQuestPanel v-if="activeQuest" :quest="activeQuest" />
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
