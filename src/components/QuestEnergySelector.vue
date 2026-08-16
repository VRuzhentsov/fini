<script setup lang="ts">
import type { Energy } from "../stores/quest";

defineProps<{
  modelValue: Energy;
  disabled?: boolean;
  testId?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: Energy];
}>();

function updateValue(event: Event) {
  emit("update:modelValue", (event.target as HTMLSelectElement).value as Energy);
}
</script>

<template>
  <label class="quest-metadata-selector">
    <span>Energy</span>
    <select
      :value="modelValue"
      :data-testid="testId"
      class="select select-bordered select-sm"
      aria-label="Quest energy"
      :disabled="disabled"
      @change="updateValue"
    >
      <option value="small">Small</option>
      <option value="medium">Medium</option>
      <option value="large">Large</option>
    </select>
  </label>
</template>
