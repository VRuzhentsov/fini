<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useQuestStore, type Quest } from "../../stores/quest";
import { SPACE_COLOR_CLASS, useSpaceStore } from "../../stores/space";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import { CalendarDaysIcon, PaperAirplaneIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import ReminderMenu from "../QuestsView/ReminderMenu.vue";

const questStore = useQuestStore();
const spaceStore = useSpaceStore();
const { ensureReminderNotificationsAllowed } = useReminderNotifications();

const title = ref("");
const description = ref("");
const selectedSpaceId = ref(spaceStore.selectedSpaceId ?? "1");
const due = ref<string | null>(null);
const dueTime = ref<string | null>(null);
const repeatRule = ref<string | null>(null);
const reminderOpen = ref(false);
const isSubmitting = ref(false);

const hasDraftContent = computed(
  () =>
    title.value.trim().length > 0 ||
    description.value.trim().length > 0 ||
    !!due.value ||
    !!dueTime.value ||
    !!repeatRule.value,
);

function defaultSpaceId() {
  return spaceStore.selectedSpaceId ?? spaceStore.spaces.find((space) => space.id === "1")?.id ?? spaceStore.spaces[0]?.id ?? "1";
}

onMounted(async () => {
  if (!spaceStore.spaces.length) {
    await spaceStore.fetchSpaces();
  }
  if (!spaceStore.spaces.some((space) => space.id === selectedSpaceId.value)) {
    selectedSpaceId.value = defaultSpaceId();
  }
});

watch(
  () => spaceStore.selectedSpaceId,
  () => {
    if (!hasDraftContent.value) {
      selectedSpaceId.value = defaultSpaceId();
    }
  },
);

const draftQuest = computed<Quest>(() => ({
  id: "__new_quest__",
  space_id: selectedSpaceId.value,
  title: title.value,
  description: description.value.trim() || null,
  status: "active",
  energy: "medium",
  priority: 1,
  pinned: false,
  due: due.value,
  due_time: dueTime.value,
  repeat_rule: repeatRule.value,
  completed_at: null,
  order_rank: 0,
  focus_enter_count: 0,
  created_at: "",
  updated_at: "",
  series_id: null,
  period_key: null,
}));

const canSubmit = computed(() => title.value.trim().length > 0 && !isSubmitting.value);

function spaceCss(spaceId: string): string {
  return SPACE_COLOR_CLASS[spaceId] ?? "";
}

function formatDate(value: string): string {
  const date = new Date(value + "T00:00:00");
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function formatTime(value: string): string {
  const [hour, minute] = value.split(":").map(Number);
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
}

function formatRepeat(value: string): string {
  try {
    const rule = JSON.parse(value);
    const labels: Record<string, string> = {
      daily: "every day",
      weekdays: "weekdays",
      weekends: "weekends",
      weekly: "every week",
      monthly: "every month",
      yearly: "every year",
    };
    return labels[rule.preset] ?? "";
  } catch {
    return "";
  }
}

const reminderText = computed(() => {
  const parts: string[] = [];
  if (due.value) {
    parts.push(`${formatDate(due.value)}${dueTime.value ? `, ${formatTime(dueTime.value)}` : ""}`);
  }
  if (repeatRule.value) {
    const repeat = formatRepeat(repeatRule.value);
    if (repeat) parts.push(repeat);
  }
  return parts.join(", ");
});

async function onReminderSave(payload: { due: string | null; due_time: string | null; repeat_rule: string | null }) {
  if (isSubmitting.value) {
    return;
  }

  if (!(await ensureReminderNotificationsAllowed(payload))) {
    return;
  }

  due.value = payload.due;
  dueTime.value = payload.due_time;
  repeatRule.value = payload.repeat_rule;
  reminderOpen.value = false;
}

function clearReminder() {
  due.value = null;
  dueTime.value = null;
  repeatRule.value = null;
}

function onTitleKeydown(event: KeyboardEvent) {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    void onSubmit();
  }
}

async function onSubmit() {
  const value = title.value.trim();
  if (!value || isSubmitting.value) return;
  const descriptionValue = description.value.trim() || null;

  isSubmitting.value = true;
  try {
    await questStore.createQuest({
      title: value,
      description: descriptionValue,
      space_id: selectedSpaceId.value,
      due: due.value,
      due_time: dueTime.value,
      repeat_rule: repeatRule.value,
    });

    title.value = "";
    description.value = "";
    clearReminder();
  } finally {
    isSubmitting.value = false;
  }
}
</script>

<template>
  <form class="new-quest-composer" @submit.prevent="onSubmit">
    <div class="new-quest-title-row">
      <span class="new-quest-check" aria-hidden="true" />
      <textarea
        v-model="title"
        data-testid="chat-input"
        class="new-quest-title"
        placeholder="New quest"
        rows="1"
        :disabled="isSubmitting"
        @keydown="onTitleKeydown"
      />
      <select
        v-model="selectedSpaceId"
        data-testid="new-quest-space"
        class="new-quest-space"
        :class="spaceCss(selectedSpaceId)"
        aria-label="Quest space"
        :disabled="isSubmitting"
      >
        <option v-for="space in spaceStore.spaces" :key="space.id" :value="space.id">
          {{ space.name }}
        </option>
      </select>
    </div>

    <textarea
      v-model="description"
      data-testid="new-quest-description"
      class="new-quest-description"
      placeholder="Description"
      rows="2"
      :disabled="isSubmitting"
    />

    <div class="new-quest-footer">
      <div class="new-quest-date-wrap">
        <button
          type="button"
          data-testid="new-quest-reminder"
          class="new-quest-date"
          :class="{ set: reminderText }"
          :disabled="isSubmitting"
          @click.stop="reminderOpen = true"
        >
          <CalendarDaysIcon />
          <span>{{ reminderText || "Date" }}</span>
        </button>
        <button
          v-if="reminderText"
          type="button"
          data-testid="new-quest-clear-reminder"
          class="new-quest-clear-date"
          aria-label="Clear date"
          :disabled="isSubmitting"
          @click.stop="clearReminder"
        >
          <XMarkIcon />
        </button>
      </div>

      <button
        type="submit"
        data-testid="chat-submit"
        class="new-quest-submit"
        :disabled="!canSubmit"
        aria-label="Create quest"
      >
        <PaperAirplaneIcon />
      </button>
    </div>
  </form>

  <ReminderMenu
    v-if="reminderOpen && !isSubmitting"
    :quest="draftQuest"
    @close="reminderOpen = false"
    @save="onReminderSave"
  />
</template>

<style scoped>
.new-quest-composer {
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

.new-quest-title-row {
  display: flex;
  align-items: center;
  gap: 0.625rem;
}

.new-quest-check {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  border: 1.5px solid var(--fg-5);
  border-radius: 4px;
}

.new-quest-title {
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

.new-quest-title::placeholder {
  color: var(--fg-5);
}

.new-quest-description {
  min-height: 2.75rem;
  padding: 0.125rem 0 0.125rem calc(18px + 0.625rem);
  color: var(--fg-2);
  font: 400 13px/1.35 Inter, Avenir, Helvetica, Arial, sans-serif;
  resize: vertical;
  background: transparent;
  border: 0;
  outline: none;
}

.new-quest-description::placeholder {
  color: var(--fg-5);
}

.new-quest-space {
  max-width: 42%;
  min-width: 6.5rem;
  height: 1.375rem;
  flex-shrink: 0;
  padding: 0 1.5rem 0 0.5rem;
  overflow: hidden;
  font-size: 0.6875rem;
  font-weight: 600;
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
  border: 0;
  border-radius: 5px;
  outline: 0;
}

.new-quest-space.space-color-personal,
.new-quest-space.space-color-work {
  color: #fff;
}

.new-quest-space.space-color-family {
  color: #1a1a1a;
}

.new-quest-space.space-color-personal {
  background: var(--space-color-personal);
}

.new-quest-space.space-color-family {
  background: var(--space-color-family);
}

.new-quest-space.space-color-work {
  background: var(--space-color-work);
}

.new-quest-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  padding-top: 0.5rem;
}

.new-quest-date-wrap {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.25rem;
}

.new-quest-date {
  display: inline-flex;
  min-width: 0;
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

.new-quest-date:hover {
  background: var(--color-base-200);
}

.new-quest-date:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.new-quest-date:disabled:hover {
  background: transparent;
}

.new-quest-date svg {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  color: var(--fg-3);
  stroke-width: 1.7;
}

.new-quest-date span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.new-quest-date.set,
.new-quest-date.set svg {
  color: var(--color-success);
}

.new-quest-clear-date,
.new-quest-submit {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  padding: 0;
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 6px;
}

.new-quest-clear-date {
  color: var(--fg-4);
}

.new-quest-clear-date:hover {
  color: var(--fg-1);
  background: var(--color-base-200);
}

.new-quest-clear-date:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.new-quest-clear-date:disabled:hover {
  color: var(--fg-4);
  background: transparent;
}

.new-quest-submit {
  color: var(--color-primary-content);
  background: var(--color-primary);
}

.new-quest-submit:hover:not(:disabled) {
  filter: brightness(0.92);
}

.new-quest-submit:disabled {
  cursor: not-allowed;
  opacity: 0.3;
}

.new-quest-clear-date svg,
.new-quest-submit svg {
  width: 18px;
  height: 18px;
  stroke-width: 1.7;
}

@media (max-width: 420px) {
  .new-quest-title-row {
    flex-wrap: wrap;
  }

  .new-quest-space {
    max-width: none;
    margin-left: calc(18px + 0.625rem);
  }
}
</style>
