<script setup lang="ts">
import { useQuestStore, type QuestWithSteps } from "../stores/quest";

const props = defineProps<{ quest: QuestWithSteps }>();
const store = useQuestStore();

async function toggleStep(id: number, done: boolean) {
  await store.updateStepDone(id, done);
}

async function completeQuest() {
  await store.updateQuestStatus(props.quest.id, "completed");
  await store.fetchActiveQuest();
}

async function abandonQuest() {
  await store.updateQuestStatus(props.quest.id, "abandoned");
  await store.fetchActiveQuest();
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

    <ul class="steps" v-if="quest.steps.length">
      <li
        v-for="step in quest.steps"
        :key="step.id"
        class="step"
        :class="{ done: step.done }"
      >
        <input
          type="checkbox"
          :checked="step.done"
          @change="toggleStep(step.id, !step.done)"
        />
        <span>{{ step.body }}</span>
      </li>
    </ul>
    <p v-else class="no-steps">No steps yet.</p>
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
  margin-bottom: 1rem;
}

.panel-header h2 {
  font-size: 1.25rem;
}

.actions {
  display: flex;
  gap: 0.5rem;
  flex-shrink: 0;
}

.steps {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.step {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.step.done span {
  text-decoration: line-through;
  opacity: 0.5;
}

.no-steps {
  opacity: 0.5;
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
}
</style>
