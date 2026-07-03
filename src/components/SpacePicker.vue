<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted } from "vue";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../stores/space";

const props = withDefaults(defineProps<{
  modelValue?: string | null;
  allowAll?: boolean;
  allLabel?: string;
  clearLabel?: string;
  testId?: string;
  ariaLabel?: string;
  disabled?: boolean;
  menuPlacement?: "bottom" | "top";
}>(), {
  allowAll: true,
  allLabel: "All spaces",
  clearLabel: "Clear space filter",
  ariaLabel: "Switch space",
  disabled: false,
  menuPlacement: "bottom",
});

const emit = defineEmits<{
  "update:modelValue": [value: string | null];
}>();

const spaceStore = useSpaceStore();
const open = ref(false);
const controlled = computed(() => props.modelValue !== undefined);
const currentSpaceId = computed(() => controlled.value ? props.modelValue ?? null : spaceStore.selectedSpaceId);

onMounted(async () => {
  if (!spaceStore.spaces.length) await spaceStore.fetchSpaces();
});

function select(id: string | null) {
  if (id === null && !props.allowAll) return;
  if (controlled.value) {
    emit("update:modelValue", id);
  } else {
    spaceStore.selectSpace(id);
  }
  open.value = false;
}

function onClickOutside(e: MouseEvent) {
  const el = (e.target as HTMLElement).closest('.space-picker');
  if (!el) open.value = false;
}

onMounted(() => window.addEventListener('click', onClickOutside));
onUnmounted(() => window.removeEventListener('click', onClickOutside));

function selectedLabel(): string {
  if (!currentSpaceId.value) return props.allLabel;
  return spaceStore.spaces.find(s => s.id === currentSpaceId.value)?.name ?? props.allLabel;
}

function selectedClass(): string {
  return currentSpaceId.value ? (SPACE_COLOR_CLASS[currentSpaceId.value] ?? "") : "";
}

function colorClass(id: string): string {
  return SPACE_COLOR_CLASS[id] ?? "";
}
</script>

<template>
  <div class="space-picker dropdown dropdown-end" :class="{ 'dropdown-open': open, 'space-picker--menu-top': menuPlacement === 'top' }">
    <div v-if="currentSpaceId" class="space-chip" :class="selectedClass()">
      <button
        type="button"
        class="space-chip-open"
        :title="ariaLabel"
        :aria-label="ariaLabel"
        :data-testid="testId"
        :disabled="disabled"
        @click.stop="open = !open"
      >
        <span>{{ selectedLabel() }}</span>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
      </button>
      <span v-if="allowAll" class="space-chip-divider" />
      <button v-if="allowAll" type="button" class="space-chip-clear" :aria-label="clearLabel" :disabled="disabled" @click.stop="select(null)">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </div>
    <button
      v-else
      type="button"
      class="space-picker-all"
      :aria-label="ariaLabel"
      :data-testid="testId"
      :disabled="disabled"
      @click.stop="open = !open"
    >
      {{ allLabel }}
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
    </button>
    <ul v-if="open" class="space-menu">
      <li v-for="space in spaceStore.spaces" :key="space.id">
        <button
          type="button"
          class="space-menu-item"
          :class="{ active: currentSpaceId === space.id }"
          :data-space-id="space.id"
          :disabled="disabled"
          @click="select(space.id)"
        >
          <span class="space-dot" :class="colorClass(space.id)" />
          <span>{{ space.name }}</span>
        </button>
      </li>
      <li v-if="allowAll" class="space-menu-separator" />
      <li v-if="allowAll">
        <button type="button" class="space-menu-item" :class="{ active: !currentSpaceId }" :disabled="disabled" @click="select(null)">{{ clearLabel }}</button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.space-picker { position: relative; }

.space-picker-all,
.space-chip {
  display: inline-flex;
  align-items: center;
  min-height: 2rem;
  font-size: 0.875rem;
  font-weight: 600;
  border-radius: 999px;
}

.space-picker-all {
  gap: 0.375rem;
  padding: 0.375rem 0.75rem;
  color: var(--fg-2);
  cursor: pointer;
  background: transparent;
  border: 1px dashed var(--color-border-softer);
}

.space-picker-all:hover { color: var(--fg-1); background: var(--color-base-200); }

.space-chip {
  overflow: hidden;
  border: 0;
  max-width: 100%;
}

.space-chip.space-color-personal,
.space-chip.space-color-work { color: #fff; }
.space-chip.space-color-family { color: #1a1a1a; }
.space-chip.space-color-personal { background: var(--space-color-personal); }
.space-chip.space-color-family { background: var(--space-color-family); }
.space-chip.space-color-work { background: var(--space-color-work); }

.space-chip-open,
.space-chip-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 2rem;
  padding: 0 0.625rem;
  color: inherit;
  cursor: pointer;
  background: transparent;
  border: 0;
}

.space-chip-open {
  gap: 0.25rem;
  min-width: 0;
}

.space-chip-open span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.space-chip-open:hover,
.space-chip-clear:hover { background: rgba(255, 255, 255, 0.18); }
.space-chip.space-color-family .space-chip-open:hover,
.space-chip.space-color-family .space-chip-clear:hover { background: rgba(0, 0, 0, 0.12); }

.space-chip-divider {
  width: 1px;
  height: 1rem;
  background: currentColor;
  opacity: 0.35;
}

svg {
  width: 0.875rem;
  height: 0.875rem;
  fill: none;
  stroke: currentColor;
  stroke-width: 2.4;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.space-menu {
  position: absolute;
  right: 0;
  z-index: 50;
  width: 12rem;
  padding: 0.375rem;
  margin-top: 0.5rem;
  list-style: none;
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
  box-shadow: var(--shadow-lg);
}

.space-picker--menu-top .space-menu {
  bottom: 100%;
  margin-top: 0;
  margin-bottom: 0.5rem;
}

.space-menu-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  min-height: 2.5rem;
  padding: 0.625rem 0.75rem;
  color: var(--fg-1);
  font-size: 0.875rem;
  font-weight: 500;
  text-align: left;
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 10px;
}

.space-menu-item:hover,
.space-menu-item.active { background: var(--color-base-200); }

.space-menu-separator {
  height: 1px;
  margin: 0.375rem 0.5rem;
  background: var(--color-border-soft);
}

.space-dot {
  width: 0.875rem;
  height: 0.875rem;
  flex-shrink: 0;
  border-radius: 999px;
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.05);
}

.space-dot.space-color-personal { background: var(--space-color-personal); }
.space-dot.space-color-family { background: var(--space-color-family); }
.space-dot.space-color-work { background: var(--space-color-work); }
</style>
