<script setup lang="ts">
import { useQuestStore, type Quest } from "../stores/quest";

defineProps<{ quests: Quest[] }>();
const store = useQuestStore();

async function makeActive(id: number) {
  await store.updateQuest(id, { status: "active" });
}
</script>

<template>
  <ul class="list">
    <li v-for="quest in quests" :key="quest.id" class="item">
      <span class="title">{{ quest.title }}</span>
      <button
        v-if="quest.status !== 'active'"
        class="btn-activate"
        @click="makeActive(quest.id)"
      >
        Make active
      </button>
    </li>
  </ul>
</template>

<style scoped>
.list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.6rem 0.75rem;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.15);
}

.title {
  flex: 1;
  font-size: 0.95rem;
}

.btn-activate {
  background: none;
  border: 1px solid rgba(128, 128, 128, 0.3);
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.75rem;
  padding: 0.2rem 0.5rem;
  color: inherit;
  opacity: 0.6;
}

.btn-activate:hover {
  opacity: 1;
}
</style>
