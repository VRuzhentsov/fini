<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useQuestStore, type Quest } from "../../stores/quest";
import { useSpaceStore } from "../../stores/space";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import {
  CalendarDaysIcon,
  CheckCircleIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  PaperAirplaneIcon,
  PlusIcon,
  XMarkIcon,
} from "@heroicons/vue/24/outline";
import { newChecklistItemId, serializeChecklist, type ChecklistItem } from "../../utils/checklistMarkdown";
import ReminderMenu from "../QuestsView/ReminderMenu.vue";
import SpacePicker from "../SpacePicker.vue";

const questStore = useQuestStore();
const spaceStore = useSpaceStore();
const { ensureReminderNotificationsAllowed } = useReminderNotifications();

const title = ref("");
const description = ref("");
// Issue #128: a checklist quest reuses this same field — when on, the composer shows a live
// checkbox list instead of a plain textarea; each row (checkable while composing) becomes one
// checklist item on submit.
const isChecklistMode = ref(false);
const checklistItems = ref<ChecklistItem[]>([]);
const newChecklistItemText = ref("");
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
    checklistItems.value.length > 0 ||
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
  description: isChecklistMode.value
    ? serializeChecklist(checklistItems.value) || null
    : description.value.trim() || null,
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

function addChecklistItem() {
  const text = newChecklistItemText.value.trim();
  if (!text) return;
  checklistItems.value.push({ id: newChecklistItemId(), text, checked: false });
  newChecklistItemText.value = "";
}

function onNewChecklistItemKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter") return;
  event.preventDefault();
  addChecklistItem();
}

function toggleChecklistItemChecked(itemId: string) {
  const item = checklistItems.value.find((it) => it.id === itemId);
  if (item) item.checked = !item.checked;
}

function removeChecklistItem(itemId: string) {
  checklistItems.value = checklistItems.value.filter((it) => it.id !== itemId);
}

async function onSubmit() {
  const value = title.value.trim();
  if (!value || isSubmitting.value) return;
  if (isChecklistMode.value) addChecklistItem(); // capture whatever's still in the "add item" input
  const descriptionValue = isChecklistMode.value
    ? serializeChecklist(checklistItems.value) || null
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
    checklistItems.value = [];
    newChecklistItemText.value = "";
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
          v-if="!isChecklistMode"
          v-model="description"
          data-testid="new-quest-description"
          class="textarea textarea-ghost min-h-11 resize-none overflow-y-auto p-0 text-sm leading-snug focus:outline-none"
          placeholder="Description"
          rows="2"
          :disabled="isSubmitting"
        />

        <div v-else data-testid="new-quest-checklist" class="flex flex-col gap-0.5">
          <div
            v-for="item in checklistItems"
            :key="item.id"
            class="flex items-center gap-2 rounded-md px-1 py-1 hover:bg-base-200"
          >
            <button
              type="button"
              class="flex size-[18px] shrink-0 items-center justify-center rounded border-[1.5px] border-base-content/40 disabled:opacity-50"
              :class="item.checked ? 'border-success bg-success' : ''"
              :aria-label="item.checked ? 'Uncheck item' : 'Check item'"
              :disabled="isSubmitting"
              @click="toggleChecklistItemChecked(item.id)"
            >
              <CheckIcon v-if="item.checked" class="size-3 text-success-content" />
            </button>
            <span
              class="min-w-0 flex-1 text-sm"
              :class="item.checked ? 'text-base-content/40 line-through' : ''"
            >{{ item.text }}</span>
            <button
              type="button"
              class="flex size-6 shrink-0 items-center justify-center rounded text-base-content/40 hover:bg-base-300 hover:text-base-content/70"
              aria-label="Remove item"
              :disabled="isSubmitting"
              @click="removeChecklistItem(item.id)"
            >
              <XMarkIcon class="size-3.5" />
            </button>
          </div>

          <div class="flex items-center gap-2 px-1 py-1">
            <PlusIcon class="size-[17px] shrink-0 text-base-content/40" />
            <input
              v-model="newChecklistItemText"
              type="text"
              data-testid="new-quest-checklist-item-input"
              class="input input-ghost min-h-0 flex-1 p-0 text-sm focus:outline-none"
              placeholder="Add item"
              :disabled="isSubmitting"
              @keydown="onNewChecklistItemKeydown"
              @blur="addChecklistItem"
            />
          </div>
        </div>
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
