<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import { useQuestStore, type Quest } from "../stores/quest";
import { useSpaceStore } from "../stores/space";
import { useDeviceStore } from "../stores/device";
import ActiveQuestPanel from "../components/FocusView/ActiveQuestPanel.vue";
import NewQuestForm from "../components/FocusView/NewQuestForm.vue";
import QuestList from "../components/QuestsView/QuestList.vue";

const store = useQuestStore();
const spaceStore = useSpaceStore();
const deviceStore = useDeviceStore();

let focusRefreshTimer: number | null = null;

onMounted(() => {
  void store.fetchQuests();
  void deviceStore.hydrate();
});

watch(
  () => deviceStore.lastAppliedSyncAt,
  (next, prev) => {
    if (!next || next === prev) return;
    void store.fetchQuests();
    void spaceStore.fetchSpaces();
  },
);

onUnmounted(() => {
  if (focusRefreshTimer !== null) window.clearTimeout(focusRefreshTimer);
});

function reminderFireAt(quest: Quest): Date | null {
  if (!quest.due) return null;
  const time = quest.due_time || "09:00";
  const fireAt = new Date(`${quest.due}T${time}`);
  return Number.isNaN(fireAt.getTime()) ? null : fireAt;
}

const nextReminderRefreshAt = computed(() => {
  const now = Date.now();
  let next: Date | null = null;
  for (const quest of store.quests) {
    if (quest.status !== "active") continue;
    const fireAt = reminderFireAt(quest);
    if (!fireAt || fireAt.getTime() <= now) continue;
    if (!next || fireAt < next) next = fireAt;
  }
  return next;
});

watch(
  nextReminderRefreshAt,
  (next) => {
    if (focusRefreshTimer !== null) window.clearTimeout(focusRefreshTimer);
    focusRefreshTimer = null;
    if (!next) return;

    const delay = Math.max(0, next.getTime() - Date.now() + 250);
    focusRefreshTimer = window.setTimeout(() => {
      focusRefreshTimer = null;
      void store.fetchQuests();
    }, Math.min(delay, 2_147_483_647));
  },
  { immediate: true },
);

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

<style scoped>
.focus-section {
  min-width: 0;
}
</style>
