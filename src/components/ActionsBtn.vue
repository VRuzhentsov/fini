<script setup lang="ts">
import { EllipsisVerticalIcon } from "@heroicons/vue/24/outline";

defineOptions({ inheritAttrs: false });
</script>

<template>
  <button type="button" class="actions-btn" v-bind="$attrs">
    <EllipsisVerticalIcon />
  </button>
</template>

<style scoped>
.actions-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  padding: 0;
  color: var(--fg-3);
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 6px;
}

.actions-btn:hover:not(:disabled) {
  color: var(--fg-1);
  background: var(--color-base-200);
}

.actions-btn:disabled {
  opacity: 0.25;
  cursor: not-allowed;
}

.actions-btn svg {
  width: 18px;
  height: 18px;
  stroke-width: 1.7;
}
</style>
