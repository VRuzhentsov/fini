<script setup lang="ts">
import { computed, ref } from "vue";
import type { Energy, Priority } from "../../stores/quest";
import { type ChecklistItem } from "../../utils/checklist";
import ChecklistEditor from "../ChecklistEditor.vue";

const props = defineProps<{
  description: string;
  energy: Energy;
  priority: Priority;
  checklistItems: ChecklistItem[];
  isChecklistMode: boolean;
  disabled: boolean;
}>();

const emit = defineEmits<{
  "update:description": [value: string];
  "update:energy": [value: Energy];
  "update:priority": [value: Priority];
  "update:checklistItems": [value: ChecklistItem[]];
  "toggle-checklist-item": [itemId: string, checked: boolean];
}>();

const checklistEditorRef = ref<InstanceType<typeof ChecklistEditor> | null>(null);

const renderFlags = computed(() => ({
  descriptionField: !props.isChecklistMode,
  checklistEditor: props.isChecklistMode,
}));

function onChecklistItemToggle(itemId: string, checked: boolean) {
  emit("toggle-checklist-item", itemId, checked);
}

function flushPendingChecklistItem() {
  checklistEditorRef.value?.flushPendingItem();
}

defineExpose({ flushPendingChecklistItem });
</script>

<template>
  <section class="flex flex-col gap-2 border-t border-base-300 pt-2" data-testid="new-quest-details">
    <textarea
      v-if="renderFlags.descriptionField"
      :value="description"
      data-testid="new-quest-description"
      class="textarea textarea-ghost min-h-11 resize-none overflow-y-auto p-0 text-sm leading-snug focus:outline-none"
      placeholder="Description"
      rows="2"
      :disabled="disabled"
      @input="emit('update:description', ($event.target as HTMLTextAreaElement).value)"
    />

    <div class="grid grid-cols-2 gap-2">
      <label class="form-control text-xs">
        <span class="label-text">Energy</span>
        <select
          :value="energy"
          data-testid="new-quest-energy"
          class="select select-bordered select-sm"
          aria-label="Quest energy"
          :disabled="disabled"
          @change="emit('update:energy', ($event.target as HTMLSelectElement).value as Energy)"
        >
          <option value="small">Small</option>
          <option value="medium">Medium</option>
          <option value="large">Large</option>
        </select>
      </label>
      <label class="form-control text-xs">
        <span class="label-text">Priority</span>
        <select
          :value="priority"
          data-testid="new-quest-priority"
          class="select select-bordered select-sm"
          aria-label="Quest priority"
          :disabled="disabled"
          @change="emit('update:priority', ($event.target as HTMLSelectElement).value as Priority)"
        >
          <option value="low">Low</option>
          <option value="medium">Medium</option>
          <option value="high">High</option>
        </select>
      </label>
    </div>

    <ChecklistEditor
      v-if="renderFlags.checklistEditor"
      ref="checklistEditorRef"
      :items="checklistItems"
      mode="draft"
      data-testid="new-quest-checklist"
      add-item-test-id="new-quest-checklist-item-input"
      :disabled="disabled"
      @update:items="emit('update:checklistItems', $event)"
      @toggle-item="onChecklistItemToggle"
    />
  </section>
</template>
