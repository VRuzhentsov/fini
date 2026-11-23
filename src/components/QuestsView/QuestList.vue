<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { useQuestStore, type Quest, type QuestPlacement } from "../../stores/quest";
import QuestListItem from "./QuestListItem.vue";

const props = defineProps<{
  quests: Quest[];
  /** When true, a person can move the rows by drag-and-drop (the active lists). */
  reorderable?: boolean;
}>();

const store = useQuestStore();

// Accordion: only one row can be expanded at a time, so this is owned by the list, not the item.
const expandedId = ref<string | null>(null);

function toggle(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
}

// ── Drag-and-drop ─────────────────────────────────────────────────────────────
// A mouse drags after it moves a few pixels with the button down. A finger or
// a pen drags after a long press, so that a short move still scrolls the
// list and a long press with no move still opens the context menu.

/** How long a finger stays on a row before the row can move. */
const LONG_PRESS_MS = 350;
/** How far a pointer moves before it is a move and not a tap. */
const MOVE_SLOP_PX = 8;
const MOUSE_POINTER = "mouse";

/** A pointer that is down on a row, before the drag starts. */
interface Press {
  id: string;
  pointerId: number;
  x: number;
  y: number;
  isMouse: boolean;
  timer: number | null;
}

/** The row that moves, and the index it has in the list now. */
interface Drag {
  id: string;
  pointerId: number;
  index: number;
}

const drag = ref<Drag | null>(null);
let press: Press | null = null;
/** True after a drop, so that the click that follows does not open the row. */
let dropped = false;
const rowElements = new Map<string, HTMLElement>();

/** The quests as the list shows them: during a drag, the moved row is at its new index. */
const renderLists = computed(() => ({
  rows: rowsWithDrag(props.quests, drag.value),
}));

function rowsWithDrag(quests: Quest[], state: Drag | null): Quest[] {
  if (!state) return quests;
  const moved = quests.find((quest) => quest.id === state.id);
  if (!moved) return quests;
  const rows = quests.filter((quest) => quest.id !== state.id);
  rows.splice(state.index, 0, moved);
  return rows;
}

function rowElement(id: string) {
  return (element: unknown) => {
    if (element instanceof HTMLElement) rowElements.set(id, element);
    else rowElements.delete(id);
  };
}

function rowClass(quest: Quest): string {
  return drag.value?.id === quest.id ? "quest-row quest-row-dragging" : "quest-row";
}

function onPointerDown(event: PointerEvent, quest: Quest) {
  dropped = false;
  const startsPress = props.reorderable && !press && !drag.value && event.button === 0;
  if (!startsPress || expandedId.value === quest.id) return;
  const isMouse = event.pointerType === MOUSE_POINTER;
  press = { id: quest.id, pointerId: event.pointerId, x: event.clientX, y: event.clientY, isMouse, timer: null };
  if (!isMouse) press.timer = window.setTimeout(startDrag, LONG_PRESS_MS);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", cancelDrag);
}

function startDrag() {
  if (!press) return;
  const index = props.quests.findIndex((quest) => quest.id === press?.id);
  if (index !== -1) drag.value = { id: press.id, pointerId: press.pointerId, index };
  clearPress();
  // A finger that drags must not scroll the page.
  window.addEventListener("touchmove", blockScroll, { passive: false });
}

function onPointerMove(event: PointerEvent) {
  if (press && event.pointerId === press.pointerId) {
    const moved = Math.hypot(event.clientX - press.x, event.clientY - press.y) > MOVE_SLOP_PX;
    if (!moved) return;
    // A mouse that moves starts the drag; a finger that moves before the long press scrolls.
    if (press.isMouse) startDrag();
    else stopListening();
  }
  if (drag.value && event.pointerId === drag.value.pointerId) moveDrag(event.clientY);
}

/** Puts the moved row at the index of the row under the pointer. */
function moveDrag(clientY: number) {
  const state = drag.value;
  if (!state) return;
  const rows = renderLists.value.rows;
  const index = rows.findIndex((quest) => {
    const rect = rowElements.get(quest.id)?.getBoundingClientRect();
    return !!rect && clientY >= rect.top && clientY <= rect.bottom;
  });
  if (index !== -1 && index !== state.index) drag.value = { ...state, index };
}

function onPointerUp(event: PointerEvent) {
  const state = drag.value;
  stopListening();
  if (!state || event.pointerId !== state.pointerId) return;
  void dropRow(state);
}

/** The move of a drop: before the row after the moved row, or after the row before it. */
function placementAt(rows: Quest[], index: number): QuestPlacement | null {
  const next = rows[index + 1];
  if (next) return { before: next.id };
  const previous = rows[index - 1];
  return previous ? { after: previous.id } : null;
}

async function dropRow(state: Drag) {
  const from = props.quests.findIndex((quest) => quest.id === state.id);
  const placement = placementAt(rowsWithDrag(props.quests, state), state.index);
  if (from === state.index || !placement) return;
  dropped = true;
  await store.moveQuest(state.id, placement);
}

/** A long press with no move opens the context menu: then the row does not move. */
function onContextMenu() {
  stopListening();
}

function onClickCapture(event: MouseEvent) {
  if (!dropped) return;
  dropped = false;
  event.stopPropagation();
  event.preventDefault();
}

function blockScroll(event: TouchEvent) {
  if (drag.value) event.preventDefault();
}

function clearPress() {
  if (press?.timer !== null && press?.timer !== undefined) window.clearTimeout(press.timer);
  press = null;
}

function cancelDrag() {
  stopListening();
}

function stopListening() {
  clearPress();
  drag.value = null;
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", cancelDrag);
  window.removeEventListener("touchmove", blockScroll);
}

onUnmounted(stopListening);
</script>

<template>
  <ul class="flex flex-col gap-1" @click.capture="onClickCapture">
    <li
      v-for="quest in renderLists.rows"
      :key="quest.id"
      :ref="rowElement(quest.id)"
      :class="rowClass(quest)"
      @pointerdown="onPointerDown($event, quest)"
      @contextmenu="onContextMenu"
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
  -webkit-touch-callout: none;
}

.quest-row-dragging {
  opacity: 0.85;
  box-shadow: 0 4px 12px rgb(0 0 0 / 0.2);
  border-radius: var(--radius-box, 0.5rem);
  position: relative;
  z-index: 1;
}
</style>
