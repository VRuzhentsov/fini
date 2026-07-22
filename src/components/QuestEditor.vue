<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { ChecklistActivity, Quest, UpdateQuestInput } from "../stores/quest";
import { SPACE_COLOR_CLASS } from "../stores/space";
import { parseChecklist } from "../utils/checklistMarkdown";
import {
  PaperClipIcon,
  TagIcon,
  FlagIcon,
  ClockIcon,
  ChevronUpIcon,
  ExclamationCircleIcon,
  CheckIcon,
  PlusIcon,
  XMarkIcon,
} from "@heroicons/vue/24/outline";
import ActionsBtn from "./ActionsBtn.vue";

const props = defineProps<{
  quest: Quest;
  spaceName: string;
  isFocus?: boolean;
  priorityColor: string;
  priorityLabel: string;
  reminderText?: string;
  timestampText?: string;
  /** Post-completion audit trail (issue #128) — only meaningful for checklist quests. */
  checklistActivity?: ChecklistActivity[];
  /** Set when this occurrence belongs to a series and the checklist is being edited, so the
   * caller can present the "This occurrence" / "This and future occurrences" scope choice. */
  isRecurring?: boolean;
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
  toggleChecklistItem: [itemId: string, checked: boolean];
  addChecklistItem: [text: string];
  removeChecklistItem: [itemId: string];
}>();

const title = ref(props.quest.title);
const description = ref(props.quest.description ?? "");
const newItemText = ref("");

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

const checklistItems = computed(() => parseChecklist(props.quest.description));
const checklistDone = computed(() => checklistItems.value.filter((it) => it.checked).length);
const checklistTotal = computed(() => checklistItems.value.length);
const checklistProgressPct = computed(() =>
  checklistTotal.value === 0 ? 0 : (checklistDone.value / checklistTotal.value) * 100,
);

function onAddItem() {
  const value = newItemText.value.trim();
  if (!value) return;
  emit("addChecklistItem", value);
  newItemText.value = "";
}

function onAddItemKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter") return;
  event.preventDefault();
  onAddItem();
}

function formatActivityTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

// Issue #128: checklist section render decisions, per fini-frontend's renderFlags convention.
const renderFlags = computed(() => ({
  checklist: props.quest.is_checklist,
  checklistRecurringBadge: props.quest.is_checklist && !!props.isRecurring,
  checklistEditable: props.quest.is_checklist && props.quest.status === "active",
  checklistAudit: props.quest.is_checklist && !!props.checklistActivity?.length,
  prose: !props.quest.is_checklist && props.quest.status === "active",
  proseReadonly: !props.quest.is_checklist && props.quest.status !== "active" && !!props.quest.description,
}));
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

    <!-- Checklist (issue #128): a checklist quest's description IS the checklist — it replaces
         the prose textarea entirely rather than sitting alongside it. -->
    <div v-if="renderFlags.checklist" class="quest-editor-checklist">
      <div class="quest-editor-checklist-header">
        <span class="quest-editor-checklist-label">Checklist</span>
        <span
          v-if="renderFlags.checklistRecurringBadge"
          class="quest-editor-checklist-repeat"
          title="This quest repeats"
        >repeats</span>
        <span class="quest-editor-checklist-progress">
          <span
            class="quest-editor-checklist-progress-fill"
            :style="{ width: checklistProgressPct + '%' }"
          />
        </span>
        <span class="quest-editor-checklist-count">{{ checklistDone }}/{{ checklistTotal }}</span>
      </div>

      <div class="quest-editor-checklist-items">
        <div v-for="item in checklistItems" :key="item.id" class="quest-editor-checklist-item">
          <button
            class="quest-editor-checklist-box"
            :class="{ checked: item.checked }"
            :aria-label="item.checked ? 'Uncheck item' : 'Check item'"
            @click.stop="emit('toggleChecklistItem', item.id, !item.checked)"
          >
            <CheckIcon v-if="item.checked" />
          </button>
          <span class="quest-editor-checklist-text" :class="{ checked: item.checked }">{{ item.text }}</span>
          <button
            v-if="renderFlags.checklistEditable"
            class="quest-editor-checklist-remove"
            aria-label="Remove item"
            @click.stop="emit('removeChecklistItem', item.id)"
          >
            <XMarkIcon />
          </button>
        </div>

        <div v-if="renderFlags.checklistEditable" class="quest-editor-checklist-add">
          <PlusIcon />
          <input
            v-model="newItemText"
            placeholder="Add item"
            @keydown="onAddItemKeydown"
            @blur="onAddItem"
          />
        </div>
      </div>

      <div v-if="renderFlags.checklistAudit" class="quest-editor-checklist-audit">
        <div v-for="entry in checklistActivity" :key="entry.id" class="quest-editor-checklist-audit-row">
          <ClockIcon />
          <span class="quest-editor-checklist-audit-detail">{{ entry.detail }}</span>
          <span class="quest-editor-checklist-audit-time">{{ formatActivityTime(entry.created_at) }}</span>
        </div>
      </div>
    </div>

    <textarea
      v-else-if="renderFlags.prose"
      v-model="description"
      class="quest-editor-desc"
      placeholder="Description"
      rows="2"
      @blur="saveDescription"
    />
    <p v-else-if="renderFlags.proseReadonly" class="quest-editor-desc readonly">{{ quest.description }}</p>

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
        <ActionsBtn title="More" @click.stop="emit('more', $event)" />
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

/* ── Checklist (issue #128) ─────────────────────────────────────────── */

.quest-editor-checklist {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.quest-editor-checklist-header {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  padding: 2px 0 6px;
}

.quest-editor-checklist-label {
  font: 600 11px var(--font-sans);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--fg-3);
}

.quest-editor-checklist-repeat {
  flex-shrink: 0;
  padding: 1px 6px;
  font-size: 10px;
  font-weight: 600;
  color: var(--fg-3);
  background: var(--color-base-200);
  border-radius: 999px;
}

.quest-editor-checklist-progress {
  flex: 1;
  min-width: 0;
  height: 3px;
  display: block;
  overflow: hidden;
  background: var(--color-base-200);
  border-radius: 999px;
}

.quest-editor-checklist-progress-fill {
  display: block;
  height: 100%;
  background: var(--color-success);
  transition: width var(--dur-normal);
}

.quest-editor-checklist-count {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--fg-3);
}

.quest-editor-checklist-items {
  display: flex;
  flex-direction: column;
}

.quest-editor-checklist-item {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  min-height: 36px;
  padding: 2px 4px;
  border-radius: 6px;
}

.quest-editor-checklist-item:hover { background: var(--color-base-200); }

.quest-editor-checklist-box {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  padding: 0;
  cursor: pointer;
  background: transparent;
  border: 1.5px solid var(--fg-5);
  border-radius: 4px;
}

.quest-editor-checklist-box:hover { border-color: var(--fg-3); }
.quest-editor-checklist-box.checked {
  background: var(--color-success);
  border-color: var(--color-success);
}
.quest-editor-checklist-box svg { width: 11px; height: 11px; color: #fff; stroke-width: 3; }

.quest-editor-checklist-text {
  flex: 1;
  min-width: 0;
  font-size: 14px;
  color: var(--fg-1);
  overflow-wrap: break-word;
}

.quest-editor-checklist-text.checked {
  color: var(--fg-4);
  text-decoration: line-through;
}

.quest-editor-checklist-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  padding: 0;
  color: var(--fg-6);
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 6px;
}

.quest-editor-checklist-remove:hover { color: var(--fg-2); background: var(--color-base-200); }
.quest-editor-checklist-remove svg { width: 13px; height: 13px; stroke-width: 2; }

.quest-editor-checklist-add {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  min-height: 36px;
  padding: 2px 4px;
}

.quest-editor-checklist-add svg {
  width: 17px;
  height: 17px;
  flex-shrink: 0;
  color: var(--fg-5);
  stroke-width: 2;
}

.quest-editor-checklist-add input {
  flex: 1;
  min-width: 0;
  font-size: 14px;
  color: var(--fg-1);
  background: transparent;
  border: 0;
  outline: none;
}

.quest-editor-checklist-add input::placeholder { color: var(--fg-5); }

.quest-editor-checklist-audit {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 4px;
  padding: 0.5rem 0.625rem;
  background: var(--color-base-200);
  border-radius: 8px;
}

.quest-editor-checklist-audit-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--fg-3);
}

.quest-editor-checklist-audit-row svg { width: 12px; height: 12px; flex-shrink: 0; stroke-width: 1.7; }
.quest-editor-checklist-audit-detail { flex: 1; min-width: 0; }
.quest-editor-checklist-audit-time { font-family: var(--font-mono); font-size: 10px; color: var(--fg-4); }
</style>
