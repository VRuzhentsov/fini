<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef } from "vue";
import type { Energy, Priority } from "../stores/quest";

type Kind = "energy" | "priority";

const props = defineProps<{
  kind: Kind;
  modelValue: Energy | Priority;
  disabled?: boolean;
  readonly?: boolean;
  testId?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: Energy | Priority];
}>();

// Tap cycles low→mid→high→low. Hold (or right-click) opens a 3-item popover instead.
const LEVELS: Record<Kind, { values: readonly string[]; labels: readonly string[] }> = {
  energy: { values: ["small", "medium", "large"], labels: ["Small", "Medium", "Large"] },
  priority: { values: ["low", "medium", "high"], labels: ["Low", "Medium", "High"] },
};
const HOLD_MS = 420;

const levelIndex = computed(() => LEVELS[props.kind].values.indexOf(props.modelValue));
const levelLabel = computed(() => LEVELS[props.kind].labels[levelIndex.value]);
const levelClass = computed(() => ["qmeta-low", "qmeta-mid", "qmeta-high"][levelIndex.value]);
const kindLabel = computed(() => (props.kind === "energy" ? "Energy" : "Priority"));
const title = computed(() => `${kindLabel.value} · ${levelLabel.value}`);
const ariaLabel = computed(() => `Quest ${props.kind}: ${levelLabel.value}`);

const root = useTemplateRef<HTMLElement>("root");
const popoverOpen = ref(false);
const holding = ref(false);
let holdTimer: number | null = null;
let held = false;

function cycle() {
  const values = LEVELS[props.kind].values;
  emit("update:modelValue", values[(levelIndex.value + 1) % values.length] as Energy | Priority);
}

function selectLevel(index: number) {
  emit("update:modelValue", LEVELS[props.kind].values[index] as Energy | Priority);
  popoverOpen.value = false;
}

function startHold() {
  if (props.disabled) return;
  held = false;
  holding.value = true;
  holdTimer = window.setTimeout(() => {
    held = true;
    holding.value = false;
    popoverOpen.value = true;
  }, HOLD_MS);
}

function endHold() {
  if (holdTimer !== null) {
    window.clearTimeout(holdTimer);
    holdTimer = null;
  }
  holding.value = false;
  if (!held && !props.disabled) cycle();
}

function cancelHold() {
  if (holdTimer !== null) {
    window.clearTimeout(holdTimer);
    holdTimer = null;
  }
  holding.value = false;
}

function openPopoverFromContextMenu(event: MouseEvent) {
  if (props.disabled) return;
  event.preventDefault();
  popoverOpen.value = true;
}

function onDocumentClick(event: MouseEvent) {
  if (popoverOpen.value && root.value && !root.value.contains(event.target as Node)) {
    popoverOpen.value = false;
  }
}

onMounted(() => document.addEventListener("click", onDocumentClick));
onBeforeUnmount(() => document.removeEventListener("click", onDocumentClick));
</script>

<template>
  <span v-if="readonly" class="qmeta-ro" :class="levelClass" :title="title" :aria-label="ariaLabel">
    <svg v-if="kind === 'energy'" viewBox="0 0 24 24"><path d="M13 2 3 14h7l-1 8 10-12h-7z" /></svg>
    <svg v-else viewBox="0 0 24 24"><path d="M4 21V4m0 0h11l-2 4 2 4H4" /></svg>
  </span>

  <span v-else ref="root" class="qmeta">
    <button
      type="button"
      class="qmeta-btn"
      :class="[levelClass, { 'qmeta-holding': holding }]"
      :data-testid="testId"
      :disabled="disabled"
      :title="title"
      :aria-label="ariaLabel"
      aria-haspopup="true"
      :aria-expanded="popoverOpen"
      @pointerdown.prevent="startHold"
      @pointerup="endHold"
      @pointerleave="cancelHold"
      @pointercancel="cancelHold"
      @contextmenu="openPopoverFromContextMenu"
    >
      <svg v-if="kind === 'energy'" viewBox="0 0 24 24"><path d="M13 2 3 14h7l-1 8 10-12h-7z" /></svg>
      <svg v-else viewBox="0 0 24 24"><path d="M4 21V4m0 0h11l-2 4 2 4H4" /></svg>
      <span class="qmeta-ring" />
    </button>

    <div v-if="popoverOpen" class="qmeta-pop" role="menu">
      <div class="qmeta-pop-h">{{ kindLabel }}</div>
      <button
        v-for="(label, index) in LEVELS[kind].labels"
        :key="label"
        type="button"
        role="menuitem"
        :aria-selected="index === levelIndex"
        @click="selectLevel(index)"
      >{{ label }}</button>
    </div>
  </span>
</template>

<style scoped>
.qmeta {
  position: relative;
  display: inline-flex;
}

.qmeta-btn {
  position: relative;
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
  -webkit-tap-highlight-color: transparent;
}

.qmeta-btn:hover:not(:disabled) { background: var(--color-base-200); }
.qmeta-btn:disabled { opacity: 0.25; cursor: not-allowed; }
.qmeta-btn svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }

.qmeta-btn.qmeta-low { color: var(--color-success); }
.qmeta-btn.qmeta-mid { color: var(--fg-3); }
.qmeta-btn.qmeta-high { color: var(--color-error); }

.qmeta-ring {
  position: absolute;
  inset: 2px;
  border-radius: 5px;
  box-shadow: 0 0 0 2px var(--color-primary) inset;
  opacity: 0;
  transition: opacity 120ms;
  pointer-events: none;
}

.qmeta-btn.qmeta-holding .qmeta-ring { opacity: 1; }

.qmeta-pop {
  position: absolute;
  z-index: 30;
  right: 0;
  bottom: calc(100% + 6px);
  display: flex;
  flex-direction: column;
  min-width: 126px;
  padding: 4px;
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 10px;
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.18);
}

.qmeta-pop-h {
  padding: 6px 8px 4px;
  color: var(--fg-4);
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.07em;
}

.qmeta-pop button {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 8px;
  color: var(--fg-2);
  font: 500 12.5px/1 Inter, Avenir, Helvetica, Arial, sans-serif;
  text-align: left;
  background: transparent;
  border: 0;
  border-radius: 7px;
  cursor: pointer;
}

.qmeta-pop button:hover { color: var(--fg-1); background: var(--color-base-200); }
.qmeta-pop button[aria-selected="true"] { color: var(--fg-1); font-weight: 600; }
.qmeta-pop button[aria-selected="true"]::after {
  content: "";
  flex-shrink: 0;
  width: 5px;
  height: 5px;
  margin-left: auto;
  background: var(--color-primary);
  border-radius: 50%;
}

.qmeta-ro {
  display: inline-flex;
  align-items: center;
  color: var(--fg-4);
}

.qmeta-ro svg { width: 13px; height: 13px; stroke: currentColor; fill: none; stroke-width: 1.9; stroke-linecap: round; stroke-linejoin: round; }
.qmeta-ro.qmeta-low { color: var(--color-success); }
.qmeta-ro.qmeta-mid { color: var(--fg-4); }
.qmeta-ro.qmeta-high { color: var(--color-error); }
</style>
