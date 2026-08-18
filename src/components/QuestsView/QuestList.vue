<script setup lang="ts">
import { ref } from "vue";
import type { Quest } from "../../stores/quest";
import QuestListItem from "./QuestListItem.vue";

defineProps<{
  quests: Quest[];
}>();

// Accordion: only one row can be expanded at a time, so this is owned by the list, not the item.
const expandedId = ref<string | null>(null);

function toggle(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
}
</script>

<template>
  <ul class="flex flex-col gap-1">
    <li
      v-for="quest in quests"
      :key="quest.id"
      class="quest-row"
    >
      <QuestListItem
        :quest="quest"
        :expanded="expandedId === quest.id"
        @toggle="toggle(quest.id)"
      />
    </li>
  </ul>
</template>

<style scoped>
.quest-row {
  list-style: none;
}
</style>
