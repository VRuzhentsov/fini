<script setup lang="ts">
import { useQuestStore, type Quest } from "../stores/quest";

const props = defineProps<{ quest: Quest }>();
const store = useQuestStore();

async function completeQuest() {
  await store.updateQuest(props.quest.id, { status: "completed" });
}

async function abandonQuest() {
  await store.updateQuest(props.quest.id, { status: "abandoned" });
}
</script>

<template>
  <div class="panel">
    <div class="panel-header">
      <h2>{{ quest.title }}</h2>
      <div class="actions">
        <button class="btn-secondary" @click="abandonQuest">Abandon</button>
        <button class="btn-primary" @click="completeQuest">Complete</button>
      </div>
    </div>
    <p v-if="quest.description" class="description">{{ quest.description }}</p>
  </div>
</template>

<style scoped>
.panel {
  border: 1px solid rgba(128, 128, 128, 0.2);
  border-radius: 8px;
  padding: 1.25rem;
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 1rem;
}

.panel-header h2 {
  font-size: 1.25rem;
}

.actions {
  display: flex;
  gap: 0.5rem;
  flex-shrink: 0;
}

.description {
  margin-top: 0.75rem;
  opacity: 0.7;
  font-size: 0.875rem;
}

.btn-primary {
  padding: 0.4rem 0.9rem;
  border-radius: 6px;
  border: none;
  background: #646cff;
  color: white;
  cursor: pointer;
  font-size: 0.875rem;
}

.btn-secondary {
  padding: 0.4rem 0.9rem;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.3);
  background: transparent;
  cursor: pointer;
  font-size: 0.875rem;
  color: inherit;
}
</style>
