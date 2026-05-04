<script setup lang="ts">
import { ref } from "vue";
import { useQuestStore, type Quest } from "../../stores/quest";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../../stores/space";
import { useContextMenu } from "../../composables/useContextMenu";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import QuestEditor from "../QuestEditor.vue";
import ReminderMenu from "../QuestsView/ReminderMenu.vue";

const props = defineProps<{ quest: Quest }>();
const store = useQuestStore();
const spaceStore = useSpaceStore();
const contextMenu = useContextMenu();
const { ensureReminderNotificationsAllowed } = useReminderNotifications();
const expanded = ref(false);
const reminderOpen = ref(false);
const HOLD_MS = 900;
let holdTimer: number | null = null;
let menuHoldTimer: number | null = null;

function onContextMenu(e: MouseEvent) {
  const moveItems = spaceStore.spaces
    .filter((s) => s.id !== props.quest.space_id)
    .map((s) => ({
      label: s.name,
      action: () => store.updateQuest(props.quest.id, { space_id: s.id }),
    }));

  const moveItem =
    moveItems.length > 0
      ? { label: "Move to space", children: moveItems }
      : { label: "Move to space", disabled: true };

  contextMenu.open(e, [
    { label: "Complete", action: () => store.updateQuest(props.quest.id, { status: "completed" }) },
    { label: "Set Focus", action: () => store.setFocusQuest(props.quest.id) },
    moveItem,
    { separator: true },
    { label: "Abandon", action: () => store.updateQuest(props.quest.id, { status: "abandoned" }) },
    { label: "Delete", action: () => store.deleteQuest(props.quest.id) },
  ]);
}

function spaceName(): string {
  return spaceStore.spaces.find((s) => s.id === props.quest.space_id)?.name ?? "";
}

function spaceCss(): string {
  return SPACE_COLOR_CLASS[props.quest.space_id] ?? "";
}

function startHold(event: PointerEvent) {
  const button = event.currentTarget as HTMLButtonElement;
  button.setPointerCapture(event.pointerId);
  button.classList.remove("released", "done");
  button.classList.add("active");
  holdTimer = window.setTimeout(() => {
    holdTimer = null;
    button.classList.remove("active");
    button.classList.add("done");
    void abandonQuest();
    window.setTimeout(() => button.classList.remove("done"), 700);
  }, HOLD_MS);
}

function endHold(event: PointerEvent) {
  const button = event.currentTarget as HTMLButtonElement;
  if (holdTimer === null) return;
  window.clearTimeout(holdTimer);
  holdTimer = null;
  button.classList.remove("active");
  button.classList.add("released");
  window.setTimeout(() => button.classList.remove("released"), 220);
}

function startMenuHold(event: PointerEvent) {
  const button = event.currentTarget as HTMLButtonElement;
  button.setPointerCapture(event.pointerId);
  button.classList.remove("released", "done");
  button.classList.add("active");
  menuHoldTimer = window.setTimeout(() => {
    menuHoldTimer = null;
    button.classList.remove("active");
    button.classList.add("done");
    onContextMenu(event as unknown as MouseEvent);
    window.setTimeout(() => button.classList.remove("done"), 700);
  }, 700);
}

function endMenuHold(event: PointerEvent) {
  const button = event.currentTarget as HTMLButtonElement;
  if (menuHoldTimer === null) return;
  window.clearTimeout(menuHoldTimer);
  menuHoldTimer = null;
  button.classList.remove("active");
  button.classList.add("released");
  window.setTimeout(() => button.classList.remove("released"), 220);
}

async function completeQuest() {
  await store.updateQuest(props.quest.id, { status: "completed" });
}

async function abandonQuest() {
  await store.updateQuest(props.quest.id, { status: "abandoned" });
}

async function onReminderSave(payload: { due: string | null; due_time: string | null; repeat_rule: string | null }) {
  if (!(await ensureReminderNotificationsAllowed(payload))) {
    return;
  }

  await store.updateQuest(props.quest.id, payload);
  reminderOpen.value = false;
}
</script>

<template>
  <QuestEditor
    v-if="expanded"
    :quest="quest"
    :space-name="spaceName()"
    is-focus
    :priority-color="'oklch(var(--color-warning))'"
    :priority-label="'Priority'"
    :reminder-text="quest.due ? 'Date set' : 'Date'"
    @contextmenu="onContextMenu"
    @update="store.updateQuest(quest.id, $event)"
    @complete="completeQuest"
    @restore="store.updateQuest(quest.id, { status: 'active' })"
    @set-focus="store.setFocusQuest(quest.id)"
    @collapse="expanded = false"
    @open-reminder="reminderOpen = true"
    @cycle-priority="store.updateQuest(quest.id, { priority: quest.priority >= 4 ? 1 : quest.priority + 1 })"
    @more="onContextMenu"
  />

  <div v-else class="active-quest-card" @contextmenu="onContextMenu">
    <div class="active-quest-top">
      <button class="active-quest-title" @click="expanded = true">{{ quest.title }}</button>
      <div class="active-quest-meta">
        <span class="badge badge-xs active-quest-space" :class="spaceCss()">{{ spaceName() }}</span>
        <button
          class="hold-ring-menu"
          title="Hold for more"
          aria-label="Hold for more actions"
          @pointerdown.prevent="startMenuHold"
          @pointerup="endMenuHold"
          @pointerleave="endMenuHold"
          @pointercancel="endMenuHold"
        >
          <span class="ring-bg" />
          <svg class="ring" viewBox="0 0 36 36" aria-hidden="true">
            <circle class="track" cx="18" cy="18" r="16" />
            <circle class="fg" cx="18" cy="18" r="16" />
          </svg>
          <span class="kebab-glyph"><span /></span>
        </button>
      </div>
    </div>

    <div class="active-quest-actions">
      <button
        class="hold-action abandon"
        aria-label="Abandon (hold)"
        @pointerdown.prevent="startHold"
        @pointerup="endHold"
        @pointerleave="endHold"
        @pointercancel="endHold"
      >
        <span class="hold-fill" />
        <span class="hold-glyph x" />
      </button>
      <button
        class="hold-action complete"
        aria-label="Complete"
        @click="completeQuest"
      >
        <span class="hold-glyph check" />
      </button>
    </div>
  </div>

  <ReminderMenu
    v-if="reminderOpen"
    :quest="quest"
    @close="reminderOpen = false"
    @save="onReminderSave"
  />
</template>

<style scoped>
.active-quest-card {
  padding: 1rem 0.875rem 0.875rem;
  color: var(--fg-1);
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06), 0 6px 16px rgba(0, 0, 0, 0.05);
}

.active-quest-top {
  display: flex;
  align-items: flex-start;
  gap: 0.625rem;
}

.active-quest-title {
  flex: 1;
  min-width: 0;
  padding: 0.125rem 0;
  color: inherit;
  font-size: 1rem;
  font-weight: 600;
  line-height: 1.3;
  letter-spacing: -0.01em;
  text-align: left;
  cursor: pointer;
  background: transparent;
  border: 0;
}

.active-quest-title:hover { color: var(--fg-2); }

.active-quest-meta {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  flex-shrink: 0;
}

.active-quest-space { border-radius: 5px; }

.hold-ring-menu {
  --ring-color: var(--fg-2);
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  padding: 0;
  cursor: pointer;
  background: transparent !important;
  border: 0 !important;
  outline: 0 !important;
  box-shadow: none !important;
  border-radius: 999px;
  appearance: none;
  -webkit-appearance: none;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
}

.hold-ring-menu:focus,
.hold-ring-menu:active,
.hold-ring-menu:focus-visible {
  background: transparent !important;
  border: 0 !important;
  outline: 0 !important;
  box-shadow: none !important;
}

.hold-ring-menu .ring-bg {
  position: absolute;
  inset: 0;
  background: transparent;
  border-radius: inherit;
  transition: transform 120ms, background-color 160ms;
}

.hold-ring-menu .ring {
  position: absolute;
  inset: 0;
  transform: rotate(-90deg);
  overflow: visible;
  pointer-events: none;
  background: transparent !important;
  border: 0 !important;
  outline: 0 !important;
  box-shadow: none !important;
}

.hold-ring-menu circle {
  fill: none;
  stroke-width: 2.5;
  stroke-linecap: round;
  stroke-dasharray: 100;
  pathLength: 100;
}

.hold-ring-menu .track { stroke: transparent; stroke-dashoffset: 0; }
.hold-ring-menu .fg { stroke: transparent; stroke-dashoffset: 100; transition: stroke-dashoffset 120ms linear; }
.hold-ring-menu.active .fg,
.hold-ring-menu.done .fg { stroke: var(--ring-color); }
.hold-ring-menu.active .fg { animation: ring-fill 700ms linear forwards; }
.hold-ring-menu.released .fg { stroke-dashoffset: 100; transition: stroke-dashoffset 220ms cubic-bezier(0.4, 0, 0.2, 1); }
.hold-ring-menu.done .fg { stroke-dashoffset: 0; animation: none; }
.hold-ring-menu.active .ring-bg { transform: scale(0.9); }
.hold-ring-menu.done .ring-bg { background: var(--ring-color); }
.hold-ring-menu.done .kebab-glyph,
.hold-ring-menu.done .kebab-glyph::before,
.hold-ring-menu.done .kebab-glyph::after,
.hold-ring-menu.done .kebab-glyph span { background: #fff; }

@keyframes ring-fill { to { stroke-dashoffset: 0; } }

.kebab-glyph,
.kebab-glyph::before,
.kebab-glyph::after,
.kebab-glyph span {
  position: absolute;
  width: 4px;
  height: 4px;
  background: var(--fg-3);
  border-radius: 999px;
}

.kebab-glyph { z-index: 1; }
.kebab-glyph::before { content: ""; top: -7px; left: 0; }
.kebab-glyph::after { content: ""; top: 7px; left: 0; }
.kebab-glyph span { top: 0; left: 0; }

.active-quest-actions {
  display: grid;
  grid-template-columns: 1fr 3fr;
  gap: 0.625rem;
  margin-top: 0.875rem;
}

.hold-action {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  height: 3.25rem;
  overflow: hidden;
  cursor: pointer;
  border: 0;
  border-radius: 12px;
  transition: transform 120ms;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
}

.hold-action:active { transform: scale(0.97); }
.hold-action.abandon { color: var(--fg-5); background: var(--color-base-200); }
.hold-action.complete { color: #fff; background: var(--color-success); box-shadow: 0 4px 10px rgba(0, 184, 107, 0.3); }

.hold-fill {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: rgba(0, 0, 0, 0.12);
  transform: scaleX(0);
  transform-origin: left center;
  transition: transform 120ms linear;
}

.hold-action.complete .hold-fill { background: rgba(255, 255, 255, 0.25); }
.hold-action.active .hold-fill { animation: fill-bar 900ms linear forwards; }
.hold-action.released .hold-fill { transform: scaleX(0); transition: transform 220ms cubic-bezier(0.4, 0, 0.2, 1); }
.hold-action.done .hold-fill { transform: scaleX(1); animation: none; }

@keyframes fill-bar { from { transform: scaleX(0); } to { transform: scaleX(1); } }

.hold-glyph {
  position: relative;
  z-index: 1;
  width: 22px;
  height: 22px;
  pointer-events: none;
}

.hold-glyph.x::before,
.hold-glyph.x::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  width: 22px;
  height: 3px;
  background: currentColor;
  border-radius: 2px;
}

.hold-glyph.x::before { transform: translate(-50%, -50%) rotate(45deg); }
.hold-glyph.x::after { transform: translate(-50%, -50%) rotate(-45deg); }
.hold-glyph.check::before,
.hold-glyph.check::after {
  content: "";
  position: absolute;
  background: currentColor;
  border-radius: 2px;
  transform-origin: left center;
}
.hold-glyph.check::before { left: 2px; top: 11px; width: 7px; height: 3px; transform: rotate(45deg); }
.hold-glyph.check::after { left: 8px; top: 14px; width: 14px; height: 3px; transform: rotate(-45deg); }
</style>
