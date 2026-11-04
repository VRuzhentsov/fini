<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import { useQuestStore, type Quest } from "../stores/quest";
import { useSpaceStore } from "../stores/space";
import { useSpaceSyncStore } from "../stores/space-sync.store";
import ActiveQuestPanel from "../components/FocusView/ActiveQuestPanel.vue";
import NewQuestForm from "../components/FocusView/NewQuestForm.vue";
import QuestList from "../components/QuestsView/QuestList.vue";

const store = useQuestStore();
const spaceStore = useSpaceStore();
const spaceSyncStore = useSpaceSyncStore();

let focusRefreshTimer: number | null = null;

onMounted(() => {
  void store.fetchQuests();
});

watch(
  () => spaceSyncStore.lastAppliedSyncAt,
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

/** The active quests of the selected space, without the Focus quest. */
const backlog = computed(() =>
  store.activeQuests.filter(
    (q) => q.id !== store.activeQuest?.id && (!spaceStore.selectedSpaceId || q.space_id === spaceStore.selectedSpaceId),
  ),
);

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
