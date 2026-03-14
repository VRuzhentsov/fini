<script setup lang="ts">
import { useQuestStore, type Quest } from "../../stores/quest";

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
  <div class="card card-bordered bg-base-200">
    <div class="card-body gap-3">
      <div class="flex items-start justify-between gap-4">
        <h2 class="card-title text-lg">{{ quest.title }}</h2>
        <div class="flex gap-2 shrink-0">
          <button class="btn btn-sm btn-ghost" @click="abandonQuest">Abandon</button>
          <button class="btn btn-sm btn-primary" @click="completeQuest">Complete</button>
        </div>
      </div>
      <p v-if="quest.description" class="text-sm opacity-70">{{ quest.description }}</p>
    </div>
  </div>
</template>
