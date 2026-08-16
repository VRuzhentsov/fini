<script setup lang="ts">
import type { Priority } from "../stores/quest";

defineProps<{
  modelValue: Priority;
  disabled?: boolean;
  testId?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: Priority];
}>();

function updateValue(event: Event) {
  emit("update:modelValue", (event.target as HTMLSelectElement).value as Priority);
}
</script>

<template>
  <label class="quest-metadata-selector">
    <span>Priority</span>
    <select
      :value="modelValue"
      :data-testid="testId"
      class="select select-bordered select-sm"
      aria-label="Quest priority"
      :disabled="disabled"
      @change="updateValue"
    >
      <option value="low">Low</option>
      <option value="medium">Medium</option>
      <option value="high">High</option>
    </select>
  </label>
</template>
