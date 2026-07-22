<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useQuestStore, type Quest } from "../../stores/quest";
import { useSpaceStore } from "../../stores/space";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import {
  CalendarDaysIcon,
  CheckCircleIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  PaperAirplaneIcon,
  XMarkIcon,
} from "@heroicons/vue/24/outline";
import { newChecklistItemId, serializeChecklist } from "../../utils/checklistMarkdown";
import ReminderMenu from "../QuestsView/ReminderMenu.vue";
import SpacePicker from "../SpacePicker.vue";

const questStore = useQuestStore();
const spaceStore = useSpaceStore();
const { ensureReminderNotificationsAllowed } = useReminderNotifications();

const title = ref("");
const description = ref("");
// Issue #128: a checklist quest reuses this same field — when on, each non-empty line typed
// here becomes one unchecked checklist item instead of a paragraph of prose.
const isChecklistMode = ref(false);
const selectedSpaceId = ref(spaceStore.selectedSpaceId ?? "1");
const due = ref<string | null>(null);
const dueTime = ref<string | null>(null);
const repeatRule = ref<string | null>(null);
const reminderOpen = ref(false);
const metadataExpanded = ref(false);
const isSubmitting = ref(false);
const titleInput = ref<HTMLInputElement | null>(null);

const hasMetadataDraft = computed(
  () =>
    description.value.trim().length > 0 ||
    !!due.value ||
    !!dueTime.value ||
    !!repeatRule.value,
);

const isExpanded = computed(() => metadataExpanded.value);

const hasDraftContent = computed(
  () =>
    title.value.trim().length > 0 ||
    hasMetadataDraft.value,
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
  [() => spaceStore.selectedSpaceId, hasDraftContent],
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
  is_checklist: isChecklistMode.value,
}));

const canSubmit = computed(() => title.value.trim().length > 0 && !isSubmitting.value);

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

  const reminder = {
    due: payload.due,
    due_time: payload.due ? payload.due_time : null,
    repeat_rule: payload.repeat_rule,
  };

  if (!(await ensureReminderNotificationsAllowed(reminder))) {
    return;
  }

  due.value = reminder.due;
  dueTime.value = reminder.due_time;
  repeatRule.value = reminder.repeat_rule;
  reminderOpen.value = false;
}

function clearReminder() {
  due.value = null;
  dueTime.value = null;
  repeatRule.value = null;
}

function toggleExpanded() {
  metadataExpanded.value = !metadataExpanded.value;
}

async function focusTitle() {
  await nextTick();
  titleInput.value?.focus();
}

function onReminderClose() {
  reminderOpen.value = false;
  void focusTitle();
}

function onTitleKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    void onSubmit();
  }
}

function toggleChecklistMode() {
  isChecklistMode.value = !isChecklistMode.value;
  if (isChecklistMode.value) metadataExpanded.value = true;
}

/** Each non-empty line typed while checklist mode is on becomes one unchecked item. */
function checklistDescriptionFromDraft(): string | null {
  const lines = description.value
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  if (lines.length === 0) return null;
  return serializeChecklist(
    lines.map((text) => ({ id: newChecklistItemId(), text, checked: false })),
  );
}

async function onSubmit() {
  const value = title.value.trim();
  if (!value || isSubmitting.value) return;
  const descriptionValue = isChecklistMode.value
    ? checklistDescriptionFromDraft()
    : description.value.trim() || null;

  isSubmitting.value = true;
  try {
    await questStore.createQuest({
      title: value,
      description: descriptionValue,
      // An empty checklist is still a checklist: its first item may be added later.
      is_checklist: isChecklistMode.value,
      space_id: selectedSpaceId.value,
      due: due.value,
      due_time: dueTime.value,
      repeat_rule: repeatRule.value,
    });

    title.value = "";
    description.value = "";
    isChecklistMode.value = false;
    clearReminder();
    metadataExpanded.value = false;
  } finally {
    isSubmitting.value = false;
  }
}
</script>

<template>
  <div class="chat-composer-bar fixed inset-x-0 bottom-0 z-50 w-full border-t border-base-300 bg-base-100 p-1">
    <form class="flex w-full flex-col gap-2 rounded-lg border border-base-300 bg-base-100 p-3 shadow-sm" @submit.prevent="onSubmit">
      <div class="flex items-start gap-2">
        <input
          ref="titleInput"
          v-model="title"
          type="text"
          data-testid="chat-input"
          class="input input-ghost min-h-0 flex-1 p-0 text-base font-semibold leading-tight focus:outline-none"
          placeholder="New quest"
          :disabled="isSubmitting"
          @keydown="onTitleKeydown"
        />
        <SpacePicker
          v-model="selectedSpaceId"
          class="max-w-[42%] shrink-0"
          menu-placement="top"
          test-id="new-quest-space"
          aria-label="Quest space"
          :allow-all="false"
          :disabled="isSubmitting"
        />
      </div>

      <div v-if="isExpanded" class="flex flex-col gap-2 border-t border-base-300 pt-2">
        <textarea
          v-model="description"
          data-testid="new-quest-description"
          class="textarea textarea-ghost min-h-11 resize-none overflow-y-auto p-0 text-sm leading-snug focus:outline-none"
          :placeholder="isChecklistMode ? 'One item per line' : 'Description'"
          rows="2"
          :disabled="isSubmitting"
        />
      </div>

      <div class="flex items-center justify-between gap-2 pt-1">
        <div class="flex min-w-0 flex-wrap items-center gap-1">
          <button
            type="button"
            data-testid="new-quest-reminder"
            class="btn btn-ghost btn-sm min-w-0 gap-2 px-2"
            :class="{ 'text-success': reminderText }"
            :disabled="isSubmitting"
            @click.stop="reminderOpen = true"
          >
            <CalendarDaysIcon class="size-5 shrink-0" />
            <span class="truncate">{{ reminderText || "Date" }}</span>
          </button>
          <button
            v-if="reminderText"
            type="button"
            data-testid="new-quest-clear-reminder"
            class="btn btn-ghost btn-xs btn-square"
            aria-label="Clear date"
            :disabled="isSubmitting"
            @click.stop="clearReminder"
          >
            <XMarkIcon class="size-4" />
          </button>
          <button
            type="button"
            data-testid="new-quest-checklist-toggle"
            class="btn btn-ghost btn-sm gap-1 px-2"
            :class="{ 'text-success': isChecklistMode }"
            :aria-pressed="isChecklistMode"
            title="Checklist"
            :disabled="isSubmitting"
            @click="toggleChecklistMode"
          >
            <CheckCircleIcon class="size-5" />
            <span>Checklist</span>
          </button>
          <button
            type="button"
            data-testid="new-quest-expand"
            class="btn btn-ghost btn-sm gap-1 px-2"
            :aria-expanded="isExpanded"
            :disabled="isSubmitting"
            @click="toggleExpanded"
          >
            <ChevronUpIcon v-if="isExpanded" class="size-4" />
            <ChevronDownIcon v-else class="size-4" />
            <span>{{ isExpanded ? "Less" : "More" }}</span>
          </button>
        </div>

        <button
          type="submit"
          data-testid="chat-submit"
          class="btn btn-primary btn-sm btn-square shrink-0"
          :disabled="!canSubmit"
          aria-label="Create quest"
        >
          <PaperAirplaneIcon class="size-5" />
        </button>
      </div>
    </form>
  </div>

  <ReminderMenu
    v-if="reminderOpen && !isSubmitting"
    :quest="draftQuest"
    @close="onReminderClose"
    @save="onReminderSave"
  />
</template>

<style scoped>
.chat-composer-bar {
  padding-bottom: calc(0.25rem + env(safe-area-inset-bottom));
}
</style>
