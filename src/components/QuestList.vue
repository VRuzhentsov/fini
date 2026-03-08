<script setup lang="ts">
import { useQuestStore, type Quest } from "../stores/quest";

defineProps<{ quests: Quest[] }>();
const store = useQuestStore();

async function remove(id: number) {
  await store.deleteQuest(id);
}
</script>

<template>
  <ul class="list">
    <li v-for="quest in quests" :key="quest.id" class="item">
      <span class="title">{{ quest.title }}</span>
      <span class="badge" :class="quest.status">{{ quest.status }}</span>
      <button class="delete" @click="remove(quest.id)" title="Delete">✕</button>
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

.badge {
  font-size: 0.75rem;
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
  text-transform: capitalize;
}

.badge.active { background: #646cff22; color: #646cff; }
.badge.completed { background: #22c55e22; color: #22c55e; }
.badge.abandoned { background: #f59e0b22; color: #f59e0b; }

.delete {
  background: none;
  border: none;
  cursor: pointer;
  opacity: 0.4;
  font-size: 0.75rem;
  padding: 0.2rem;
}

.delete:hover {
  opacity: 1;
}
</style>
