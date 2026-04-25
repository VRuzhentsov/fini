<script setup lang="ts">
import { ref, watch } from "vue";
import type { Quest, UpdateQuestInput } from "../stores/quest";
import { SPACE_COLOR_CLASS } from "../stores/space";
import {
  PaperClipIcon,
  TagIcon,
  FlagIcon,
  ClockIcon,
  EllipsisVerticalIcon,
  ChevronUpIcon,
  ExclamationCircleIcon,
} from "@heroicons/vue/24/outline";

const props = defineProps<{
  quest: Quest;
  spaceName: string;
  isFocus?: boolean;
  priorityColor: string;
  priorityLabel: string;
  reminderText?: string;
  timestampText?: string;
}>();

const emit = defineEmits<{
  update: [patch: UpdateQuestInput];
  complete: [];
  restore: [];
  setFocus: [];
  collapse: [];
  openReminder: [];
  cyclePriority: [];
  more: [event: MouseEvent];
}>();

const title = ref(props.quest.title);
const description = ref(props.quest.description ?? "");

watch(
  () => props.quest,
  (quest) => {
    title.value = quest.title;
    description.value = quest.description ?? "";
  },
);

function saveTitle() {
  const value = title.value.trim();
  if (!value) {
    title.value = props.quest.title;
    return;
  }
  if (value !== props.quest.title) emit("update", { title: value });
}

function saveDescription() {
  const value = description.value.trim() || null;
  if (value !== props.quest.description) emit("update", { description: value });
}

function onTitleKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter") return;
  event.preventDefault();
  (event.target as HTMLInputElement).blur();
}

function spaceCss(): string {
  return SPACE_COLOR_CLASS[props.quest.space_id] ?? "";
}
</script>

<template>
  <div class="quest-editor" @click.stop>
    <div class="quest-editor-title-row">
      <button
        v-if="quest.status !== 'active'"
        class="quest-editor-check"
        :class="{ completed: quest.status === 'completed', abandoned: quest.status === 'abandoned' }"
        aria-label="Make active"
        @click.stop="emit('restore')"
      />
      <button
        v-else
        class="quest-editor-check"
        aria-label="Complete"
        @click.stop="emit('complete')"
      />

      <input
        v-if="quest.status === 'active'"
        v-model="title"
        class="quest-editor-title"
        type="text"
        @blur="saveTitle"
        @keydown="onTitleKeydown"
      />
      <span v-else class="quest-editor-title readonly" :class="quest.status">{{ quest.title }}</span>

      <span class="quest-editor-space badge badge-xs" :class="spaceCss()">{{ spaceName }}</span>
      <span v-if="timestampText" class="quest-editor-status" :class="quest.status">
        {{ quest.status === "completed" ? "Completed" : "Abandoned" }} · {{ timestampText }}
      </span>

      <button
        v-if="quest.status === 'active'"
        class="quest-editor-icon"
        :class="{ muted: !isFocus }"
        title="Set Focus"
        @click.stop="emit('setFocus')"
      >
        <ExclamationCircleIcon />
      </button>
      <button class="quest-editor-icon" title="Collapse" @click.stop="emit('collapse')">
        <ChevronUpIcon />
      </button>
    </div>

    <textarea
      v-if="quest.status === 'active'"
      v-model="description"
      class="quest-editor-desc"
      placeholder="Description"
      rows="2"
      @blur="saveDescription"
    />
    <p v-else-if="quest.description" class="quest-editor-desc readonly">{{ quest.description }}</p>

    <div class="quest-editor-footer">
      <button
        v-if="quest.status === 'active'"
        data-testid="quest-reminder"
        class="quest-editor-date"
        title="Reminder"
        @click.stop="emit('openReminder')"
      >
        <ClockIcon />
        <span>{{ reminderText || "Date" }}</span>
      </button>
      <div v-else />

      <div class="quest-editor-actions">
        <template v-if="quest.status === 'active'">
          <button class="quest-editor-icon disabled" disabled title="Attachment"><PaperClipIcon /></button>
          <button class="quest-editor-icon disabled" disabled title="Label"><TagIcon /></button>
          <button
            class="quest-editor-icon"
            :style="{ color: priorityColor }"
            :title="priorityLabel"
            @click.stop="emit('cyclePriority')"
          >
            <FlagIcon />
          </button>
        </template>
        <button class="quest-editor-icon" title="More" @click.stop="emit('more', $event)">
          <EllipsisVerticalIcon />
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.quest-editor {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  width: 100%;
  padding: 0.75rem 0.875rem 0.625rem;
  color: var(--fg-1);
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
  box-shadow: var(--shadow-sm);
}

.quest-editor-title-row {
  display: flex;
  align-items: center;
  gap: 0.625rem;
}

.quest-editor-check {
  position: relative;
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  padding: 0;
  cursor: pointer;
  background: transparent;
  border: 1.5px solid var(--fg-5);
  border-radius: 4px;
}

.quest-editor-check:hover { border-color: var(--fg-3); }
.quest-editor-check.completed { background: var(--color-success); border-color: var(--color-success); }
.quest-editor-check.completed::before,
.quest-editor-check.completed::after {
  content: "";
  position: absolute;
  background: #fff;
  border-radius: 2px;
  transform-origin: left center;
}
.quest-editor-check.completed::before { left: 3px; top: 9px; width: 4px; height: 2px; transform: rotate(45deg); }
.quest-editor-check.completed::after { left: 5.5px; top: 10.5px; width: 9px; height: 2px; transform: rotate(-45deg); }
.quest-editor-check.abandoned::before,
.quest-editor-check.abandoned::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  width: 11px;
  height: 1.8px;
  background: var(--fg-4);
  border-radius: 2px;
}
.quest-editor-check.abandoned::before { transform: translate(-50%, -50%) rotate(45deg); }
.quest-editor-check.abandoned::after { transform: translate(-50%, -50%) rotate(-45deg); }

.quest-editor-title {
  min-width: 0;
  flex: 1;
  padding: 0.125rem 0;
  color: var(--fg-1);
  font: 600 15px/1.3 Inter, Avenir, Helvetica, Arial, sans-serif;
  letter-spacing: -0.01em;
  background: transparent;
  border: 0;
  outline: none;
}

.quest-editor-title.readonly.completed {
  text-decoration: line-through;
  color: var(--fg-4);
}

.quest-editor-title.readonly.abandoned { color: var(--fg-4); }

.quest-editor-space {
  flex-shrink: 0;
  border-radius: 5px;
}

.quest-editor-status {
  flex-shrink: 0;
  padding: 0.125rem 0.5rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.7rem;
  line-height: 1.4;
  border-radius: 6px;
}

.quest-editor-status.completed {
  color: #fff;
  background: var(--color-success);
}

.quest-editor-status.abandoned {
  color: var(--fg-4);
  border: 1px solid var(--color-border-soft);
}

.quest-editor-icon {
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

.quest-editor-icon:hover:not(:disabled) { color: var(--fg-1); background: var(--color-base-200); }
.quest-editor-icon svg { width: 18px; height: 18px; stroke-width: 1.7; }
.quest-editor-icon.muted { opacity: 0.35; }
.quest-editor-icon.disabled { opacity: 0.25; cursor: not-allowed; }

.quest-editor-desc {
  width: 100%;
  min-height: 48px;
  max-height: 200px;
  padding: 0.125rem 0;
  color: var(--fg-3);
  font: 400 13px/1.5 Inter, Avenir, Helvetica, Arial, sans-serif;
  resize: none;
  background: transparent;
  border: 0;
  outline: none;
}

.quest-editor-desc::placeholder { color: var(--fg-5); }
.quest-editor-desc.readonly { min-height: 0; margin: 0; }

.quest-editor-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  padding-top: 0.5rem;
}

.quest-editor-date {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem 0.625rem 0.375rem 0.25rem;
  color: var(--fg-1);
  font: 600 13px/1 Inter, Avenir, Helvetica, Arial, sans-serif;
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 8px;
}

.quest-editor-date:hover { background: var(--color-base-200); }
.quest-editor-date svg { width: 18px; height: 18px; color: var(--fg-3); stroke-width: 1.7; }
.quest-editor-actions { display: flex; align-items: center; gap: 0; }
</style>
