<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { ArrowPathIcon, CalendarDaysIcon } from "@heroicons/vue/24/outline";
import type { EditScope } from "../../stores/quest";

defineProps<{
  /** Title of the quest whose checklist is being edited, shown in the prompt. */
  questTitle: string;
}>();

// ADR-0012 D8: one day (an override on that occurrence), or the quest.
const emit = defineEmits<{
  close: [];
  choose: [scope: EditScope];
}>();

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onUnmounted(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <Teleport to="body">
    <div
      data-testid="recurrence-scope-backdrop"
      class="fixed inset-0 z-[199] bg-black/30"
      @click="emit('close')"
    />
    <div
      data-testid="recurrence-scope-sheet"
      class="recurrence-scope-panel fixed z-[200] bg-base-100 shadow-xl flex flex-col"
      style="padding-bottom: env(safe-area-inset-bottom)"
      @click.stop
    >
      <div class="recurrence-scope-handle-row">
        <span class="recurrence-scope-handle" />
      </div>
      <p class="recurrence-scope-title">Apply checklist change</p>
      <p class="recurrence-scope-subtitle">{{ questTitle }} repeats. Where should this change apply?</p>

      <button class="recurrence-scope-option" data-testid="recurrence-scope-day" @click="emit('choose', 'day')">
        <CalendarDaysIcon class="recurrence-scope-option-icon" />
        <span class="recurrence-scope-option-text">
          <span class="recurrence-scope-option-label">This day</span>
          <span class="recurrence-scope-option-hint">Only this day's checklist changes</span>
        </span>
      </button>

      <button class="recurrence-scope-option" data-testid="recurrence-scope-all" @click="emit('choose', 'all')">
        <ArrowPathIcon class="recurrence-scope-option-icon" />
        <span class="recurrence-scope-option-text">
          <span class="recurrence-scope-option-label">All days</span>
          <span class="recurrence-scope-option-hint">Changes the quest, so every day shows it</span>
        </span>
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
.recurrence-scope-panel {
  left: max(0.375rem, env(safe-area-inset-left));
  right: max(0.375rem, env(safe-area-inset-right));
  bottom: 0;
  padding: 0 0.375rem 0.625rem;
  color: var(--fg-1);
  border: 1px solid var(--color-border-soft);
  border-bottom: 0;
  border-radius: 16px 16px 0 0;
}

.recurrence-scope-handle-row {
  display: flex;
  justify-content: center;
  padding: 0.5rem 0 0.25rem;
}

.recurrence-scope-handle {
  display: block;
  width: 36px;
  height: 4px;
  background: var(--fg-5);
  opacity: 0.5;
  border-radius: 999px;
}

.recurrence-scope-title {
  margin: 0.25rem 0.75rem 0.125rem;
  font: 600 14px var(--font-sans);
}

.recurrence-scope-subtitle {
  margin: 0 0.75rem 0.625rem;
  font-size: 12px;
  color: var(--fg-3);
}

.recurrence-scope-option {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  width: 100%;
  min-height: 52px;
  padding: 0.625rem 0.75rem;
  color: var(--fg-1);
  text-align: left;
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 8px;
}

.recurrence-scope-option:hover { background: var(--color-base-200); }

.recurrence-scope-option-icon {
  width: 18px;
  height: 18px;
  margin-top: 2px;
  flex-shrink: 0;
  color: var(--fg-3);
  stroke-width: 1.7;
}

.recurrence-scope-option-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.recurrence-scope-option-label {
  font: 500 14px var(--font-sans);
}

.recurrence-scope-option-hint {
  font-size: 12px;
  color: var(--fg-4);
}
</style>
