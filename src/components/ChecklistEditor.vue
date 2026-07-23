<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import type { ComponentPublicInstance } from "vue";
import type { ChecklistItem } from "../utils/checklist";
import { newChecklistItemId } from "../utils/checklist";
import { CheckIcon, PlusIcon, XMarkIcon } from "@heroicons/vue/24/outline";

/**
 * - draft: composer, pre-submit — item text is always an editable input (no quest exists yet to
 *   hold-edit against).
 * - active: an existing, active quest's full editor — toggle/add/remove; rename via hold-to-edit
 *   (per the Fini App Refresh design, item text is read-only until held).
 * - compact: the Focus view's collapsed hero card — toggle and hold-to-edit only, no add/remove
 *   (no room for full CRUD in the collapsed glance view).
 * - readonly: a completed/abandoned occurrence's snapshot — fully static, toggle disabled.
 */
type ChecklistEditorMode = "draft" | "active" | "compact" | "readonly";

const props = withDefaults(
  defineProps<{
    items: ChecklistItem[];
    mode: ChecklistEditorMode;
    disabled?: boolean;
    /** Test hook for the "add item" input specifically, since the root element's own `data-testid`
     * (via attrs fallthrough) can't also reach a child element. */
    addItemTestId?: string;
  }>(),
  { disabled: false },
);

const emit = defineEmits<{
  "update:items": [items: ChecklistItem[]];
  "toggle-item": [itemId: string, checked: boolean];
  "edit-item-text": [itemId: string, text: string];
  "add-item": [text: string];
  "remove-item": [itemId: string];
}>();

const renderFlags = computed(() => ({
  canToggle: props.mode !== "readonly",
  canRemove: props.mode === "draft" || props.mode === "active",
  canAdd: props.mode === "draft" || props.mode === "active",
  alwaysEditableText: props.mode === "draft",
  holdToEdit: props.mode === "active" || props.mode === "compact",
}));

const newItemText = ref("");
const editingItemId = ref<string | null>(null);
const HOLD_MS = 600;
let holdTimer: number | null = null;

const editInputEls = new Map<string, HTMLInputElement>();
function setEditInputRef(itemId: string, el: Element | ComponentPublicInstance | null) {
  if (el) editInputEls.set(itemId, el as HTMLInputElement);
  else editInputEls.delete(itemId);
}

const addInputEl = ref<HTMLInputElement | null>(null);

async function beginEdit(itemId: string) {
  editingItemId.value = itemId;
  await nextTick();
  const el = editInputEls.get(itemId);
  el?.focus();
  el?.select();
}

function startHold(itemId: string) {
  if (!renderFlags.value.holdToEdit || props.disabled) return;
  cancelHold();
  holdTimer = window.setTimeout(() => {
    holdTimer = null;
    void beginEdit(itemId);
  }, HOLD_MS);
}

function cancelHold() {
  if (holdTimer !== null) {
    window.clearTimeout(holdTimer);
    holdTimer = null;
  }
}

function onToggle(itemId: string, checked: boolean) {
  emit("toggle-item", itemId, checked);
}

function onAdd() {
  const value = newItemText.value.trim();
  if (!value) return;
  emit("add-item", value);
  emit("update:items", [...props.items, { id: newChecklistItemId(), text: value, checked: false }]);
  newItemText.value = "";
}

function onAddKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter") return;
  event.preventDefault();
  onAdd();
}

function commitEdit(itemId: string, currentText: string, event: FocusEvent) {
  const input = event.target as HTMLInputElement;
  const value = input.value.trim();
  editingItemId.value = null;
  if (!value || value === currentText) {
    input.value = currentText;
    return;
  }
  emit("edit-item-text", itemId, value);
  emit(
    "update:items",
    props.items.map((it) => (it.id === itemId ? { ...it, text: value } : it)),
  );
}

function onEditKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter") return;
  event.preventDefault();
  if (renderFlags.value.canAdd) {
    (event.target as HTMLInputElement).blur();
    addInputEl.value?.focus();
    return;
  }
  (event.target as HTMLInputElement).blur();
}

function onRemove(itemId: string) {
  emit("remove-item", itemId);
  emit(
    "update:items",
    props.items.filter((it) => it.id !== itemId),
  );
}

/** Commits any text still sitting in the "add item" input — for callers (the composer) that
 * submit without the input ever losing focus, so a typed-but-not-yet-Entered item isn't lost. */
defineExpose({ flushPendingItem: onAdd });
</script>

<template>
  <div class="flex flex-col gap-0.5">
    <div
      v-for="item in items"
      :key="item.id"
      class="group flex items-center gap-2.5 min-h-9 rounded-md px-1 hover:bg-base-200"
    >
      <button
        type="button"
        class="relative inline-flex size-[17px] shrink-0 items-center justify-center rounded border-[1.5px] p-0"
        :class="
          item.checked
            ? 'border-success bg-success'
            : 'border-base-content/30 bg-transparent hover:border-base-content/50'
        "
        :disabled="disabled || !renderFlags.canToggle"
        :aria-label="item.checked ? 'Uncheck item' : 'Check item'"
        @click.stop="onToggle(item.id, !item.checked)"
      >
        <CheckIcon v-if="item.checked" class="size-[11px] text-success-content" stroke-width="3" />
      </button>

      <input
        v-if="renderFlags.alwaysEditableText || editingItemId === item.id"
        :ref="(el) => setEditInputRef(item.id, el)"
        class="checklist-item-text min-w-0 flex-1 bg-transparent p-0 text-sm text-base-content focus:outline-none"
        :class="{ 'text-base-content/40 line-through': item.checked }"
        :value="item.text"
        :disabled="disabled"
        type="text"
        aria-label="Checklist item text"
        @click.stop
        @blur="commitEdit(item.id, item.text, $event)"
        @keydown="onEditKeydown"
      />
      <span
        v-else
        class="checklist-item-text min-w-0 flex-1 select-none text-sm"
        :class="[
          item.checked ? 'text-base-content/40 line-through' : 'text-base-content',
          renderFlags.holdToEdit ? 'cursor-default' : '',
        ]"
        @pointerdown="startHold(item.id)"
        @pointerup="cancelHold"
        @pointerleave="cancelHold"
        @pointercancel="cancelHold"
      >{{ item.text }}</span>

      <button
        v-if="renderFlags.canRemove"
        type="button"
        class="flex size-6 shrink-0 items-center justify-center rounded text-base-content/40 opacity-0 hover:bg-base-300 hover:text-base-content/70 group-hover:opacity-100 focus-visible:opacity-100"
        aria-label="Remove item"
        :disabled="disabled"
        @click.stop="onRemove(item.id)"
      >
        <XMarkIcon class="size-3.5" />
      </button>
    </div>

    <div v-if="renderFlags.canAdd" class="flex min-h-9 items-center gap-2.5 px-1">
      <PlusIcon class="size-[15px] shrink-0 text-base-content/40" stroke-width="2" />
      <input
        ref="addInputEl"
        v-model="newItemText"
        type="text"
        placeholder="Add item"
        :data-testid="addItemTestId"
        class="min-w-0 flex-1 bg-transparent p-0 text-sm text-base-content placeholder:text-base-content/40 focus:outline-none"
        :disabled="disabled"
        @keydown="onAddKeydown"
        @blur="onAdd"
      />
    </div>
  </div>
</template>
