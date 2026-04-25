<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import type { Quest } from "../../stores/quest";
import {
  StarIcon,
  CalendarDaysIcon,
  ArrowPathIcon,
  ClockIcon,
  PlusIcon,
  XMarkIcon,
  ChevronLeftIcon,
  ChevronDoubleLeftIcon,
  ChevronDoubleRightIcon,
} from "@heroicons/vue/24/outline";

const props = defineProps<{ quest: Quest }>();
const emit = defineEmits<{
  close: [];
  save: [payload: { due: string | null; due_time: string | null; repeat_rule: string | null }];
}>();

// ── Local state ────────────────────────────────────────────────────────────────

const localDue = ref<string | null>(props.quest.due ?? null);
const localDueTime = ref<string | null>(props.quest.due_time ?? null);
const localRepeatRule = ref<string | null>(props.quest.repeat_rule ?? null);

const showTime = ref(!!props.quest.due_time);
const showRepeat = ref(false);
const sheetRef = ref<HTMLElement | null>(null);

// ── Close on outside click ─────────────────────────────────────────────────────

function handleOutsideClick(e: MouseEvent) {
  if (sheetRef.value && !e.composedPath().includes(sheetRef.value)) {
    emit("close");
  }
}

onMounted(() => {
  setTimeout(() => document.addEventListener("click", handleOutsideClick), 0);
});

onUnmounted(() => {
  document.removeEventListener("click", handleOutsideClick);
});

// ── Date helpers ───────────────────────────────────────────────────────────────

function toDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function todayDate() { return toDateStr(new Date()); }
function tomorrowDate() { const d = new Date(); d.setDate(d.getDate() + 1); return toDateStr(d); }
function nextWeekDate() {
  const d = new Date();
  d.setDate(d.getDate() + 7);
  return toDateStr(d);
}

function formatDue(due: string): string {
  const date = new Date(due + "T00:00:00");
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

// ── Inline calendar ────────────────────────────────────────────────────────────

const today = new Date();
const calYear = ref(
  localDue.value ? parseInt(localDue.value.slice(0, 4)) : today.getFullYear()
);
const calMonth = ref(
  localDue.value ? parseInt(localDue.value.slice(5, 7)) - 1 : today.getMonth()
);

const MONTH_NAMES = ["January","February","March","April","May","June","July","August","September","October","November","December"];
const DAY_LABELS = ["Su","Mo","Tu","We","Th","Fr","Sa"];

const calDays = computed(() => {
  const firstDay = new Date(calYear.value, calMonth.value, 1).getDay();
  const daysInMonth = new Date(calYear.value, calMonth.value + 1, 0).getDate();
  const cells: (number | null)[] = Array(firstDay).fill(null);
  for (let d = 1; d <= daysInMonth; d++) cells.push(d);
  while (cells.length % 7 !== 0) cells.push(null);
  return cells;
});

function calDateStr(day: number): string {
  const m = String(calMonth.value + 1).padStart(2, "0");
  const d = String(day).padStart(2, "0");
  return `${calYear.value}-${m}-${d}`;
}

function isToday(day: number): boolean {
  return calDateStr(day) === toDateStr(today);
}

function prevMonth() {
  if (calMonth.value === 0) { calMonth.value = 11; calYear.value--; }
  else calMonth.value--;
}

function nextMonth() {
  if (calMonth.value === 11) { calMonth.value = 0; calYear.value++; }
  else calMonth.value++;
}

function pickDay(day: number) {
  localDue.value = calDateStr(day);
}

// ── Repeat ─────────────────────────────────────────────────────────────────────

const REPEAT_PRESETS = [
  { value: "daily",    label: "Every day" },
  { value: "weekdays", label: "Weekdays" },
  { value: "weekends", label: "Weekends" },
  { value: "weekly",   label: "Every week" },
  { value: "monthly",  label: "Every month" },
  { value: "yearly",   label: "Every year" },
];

const currentRepeatPreset = computed<string | null>(() => {
  if (!localRepeatRule.value) return null;
  try { return JSON.parse(localRepeatRule.value).preset ?? null; } catch { return null; }
});

const repeatSummary = computed<string>(() => {
  if (!localRepeatRule.value) return "";
  try {
    const r = JSON.parse(localRepeatRule.value);
    const labels: Record<string, string> = {
      daily: "every day", weekdays: "weekdays", weekends: "weekends",
      weekly: "every week", monthly: "every month", yearly: "every year",
    };
    const preset = r.preset;
    if (preset && preset !== "none" && preset !== "custom") return labels[preset] ?? preset;
    const n = r.interval ?? 1;
    const unit = r.unit ?? "week";
    return `every ${n} ${unit}${n > 1 ? "s" : ""}`;
  } catch { return ""; }
});

function setRepeatPreset(preset: string) {
  localRepeatRule.value = JSON.stringify({ preset });
  showRepeat.value = false;
}

function clearRepeat() {
  localRepeatRule.value = null;
  showRepeat.value = false;
}

// ── Time ───────────────────────────────────────────────────────────────────────

function clampTimePart(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

const timeHour = computed({
  get: () => localDueTime.value ? parseInt(localDueTime.value.split(":")[0] ?? "09", 10) : 9,
  set: (h: number | string) => {
    const m = localDueTime.value ? (localDueTime.value.split(":")[1] ?? "00") : "00";
    const hour = clampTimePart(Number(h), 0, 23);
    localDueTime.value = `${String(hour).padStart(2, "0")}:${m}`;
  },
});

const timeMinute = computed({
  get: () => localDueTime.value ? parseInt(localDueTime.value.split(":")[1] ?? "00", 10) : 0,
  set: (m: number | string) => {
    const h = localDueTime.value ? (localDueTime.value.split(":")[0] ?? "09") : "09";
    const minute = clampTimePart(Number(m), 0, 59);
    localDueTime.value = `${h}:${String(minute).padStart(2, "0")}`;
  },
});

function normalizeHourInput(event: Event) {
  timeHour.value = (event.target as HTMLInputElement).value;
}

function normalizeMinuteInput(event: Event) {
  timeMinute.value = (event.target as HTMLInputElement).value;
}

function defaultDueTime(): string {
  const now = new Date();
  now.setHours(now.getHours() + 1);
  return `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
}

function toggleTime() {
  if (showTime.value) {
    showTime.value = false;
    localDueTime.value = null;
  } else {
    showTime.value = true;
    if (!localDueTime.value) localDueTime.value = defaultDueTime();
  }
}

// ── Actions ────────────────────────────────────────────────────────────────────

function onClear() {
  emit("save", { due: null, due_time: null, repeat_rule: null });
}

function onDone() {
  emit("save", {
    due: localDue.value,
    due_time: showTime.value ? localDueTime.value : null,
    repeat_rule: localRepeatRule.value,
  });
}
</script>

<template>
  <Teleport to="body">
    <!-- Visual backdrop -->
    <div class="fixed inset-0 z-[199] bg-black/30 pointer-events-none" />

    <!-- Sheet -->
    <div
      ref="sheetRef"
      class="reminder-panel fixed z-[200] bg-base-100 shadow-xl flex flex-col"
      style="padding-bottom: env(safe-area-inset-bottom)"
    >
      <!-- ── Repeat sub-sheet ── -->
      <template v-if="showRepeat">
        <div class="flex items-center gap-2 px-4 py-3 border-b border-base-content/10">
          <button class="btn btn-ghost btn-xs btn-square" @click="showRepeat = false">
            <ChevronLeftIcon class="size-4" />
          </button>
          <span class="font-semibold flex-1">Repeat</span>
          <button class="btn btn-ghost btn-xs" @click="clearRepeat">Clear</button>
        </div>
        <ul class="menu px-2 py-2 gap-1">
          <li v-for="preset in REPEAT_PRESETS" :key="preset.value">
            <a
              class="flex items-center"
              :class="{ 'active': currentRepeatPreset === preset.value }"
              @click="setRepeatPreset(preset.value)"
            >
              <ArrowPathIcon class="size-4 mr-2" />
              {{ preset.label }}
            </a>
          </li>
        </ul>
      </template>

      <!-- ── Main sheet ── -->
      <template v-else>
        <label class="reminder-search">
          <CalendarDaysIcon class="size-4 opacity-50" />
          <input type="text" placeholder="Type a date…" />
        </label>

        <!-- Quick picks -->
        <div class="flex gap-2 px-3 py-3 border-b border-base-content/10 flex-wrap">
          <button
            data-testid="reminder-today"
            class="btn btn-sm rounded-full gap-1"
            :class="localDue === todayDate() ? 'btn-primary' : 'btn-ghost bg-base-300'"
            @click="localDue = todayDate()"
          >
            <StarIcon class="size-3" /> Today
          </button>
          <button
            data-testid="reminder-tomorrow"
            class="btn btn-sm rounded-full gap-1"
            :class="localDue === tomorrowDate() ? 'btn-primary' : 'btn-ghost bg-base-300'"
            @click="localDue = tomorrowDate()"
          >
            <CalendarDaysIcon class="size-3" /> Tomorrow
          </button>
          <button
            data-testid="reminder-next-week"
            class="btn btn-sm rounded-full gap-1"
            :class="localDue === nextWeekDate() ? 'btn-primary' : 'btn-ghost bg-base-300'"
            @click="localDue = nextWeekDate()"
          >
            <CalendarDaysIcon class="size-3" /> Next week
          </button>
        </div>

        <div class="px-3 py-3 border-b border-base-content/10">
          <div class="mb-2 flex items-center justify-between">
            <button class="reminder-month-button" @click="prevMonth"><ChevronDoubleLeftIcon class="size-4" /></button>
            <span class="text-sm font-medium">{{ MONTH_NAMES[calMonth] }} {{ calYear }}</span>
            <button class="reminder-month-button" @click="nextMonth"><ChevronDoubleRightIcon class="size-4" /></button>
          </div>
          <div class="grid grid-cols-7 pb-2">
            <span
              v-for="label in DAY_LABELS" :key="label"
              class="text-center text-xs opacity-40 py-1"
            >{{ label }}</span>
          </div>
          <div class="grid grid-cols-7 gap-y-1">
            <button
              v-for="(day, i) in calDays" :key="i"
              :disabled="!day"
              class="h-9 w-full rounded-lg text-sm flex items-center justify-center disabled:opacity-0 transition-colors"
              :class="day ? [
                localDue === calDateStr(day) ? 'bg-primary text-primary-content font-semibold' :
                isToday(day) ? 'text-success font-semibold hover:bg-base-200' :
                'hover:bg-base-200'
              ] : []"
              @click="day && pickDay(day)"
            >{{ day }}</button>
          </div>
        </div>

        <div class="border-b border-base-content/10 flex items-center gap-2 px-4 py-2">
          <CalendarDaysIcon class="size-4 opacity-60 shrink-0" />
          <button
            v-if="localDue"
            class="btn btn-xs rounded-full btn-ghost bg-base-300 gap-1"
            @click="localDue = null"
          >
            {{ formatDue(localDue) }}
            <XMarkIcon class="size-3" />
          </button>
          <span class="flex-1 py-1 text-sm opacity-60">{{ localDue ? "Selected date" : "Choose a date above" }}</span>
        </div>

        <!-- Repeat -->
        <div class="border-b border-base-content/10">
          <button
            class="flex items-center w-full gap-3 px-4 py-3 hover:bg-base-300 transition-colors"
            @click="showRepeat = true"
          >
            <ArrowPathIcon class="size-4 opacity-60" />
            <span class="flex-1 text-sm text-left">{{ repeatSummary || "Repeat" }}</span>
            <ChevronRightIcon class="size-4 opacity-40" />
          </button>
        </div>

        <!-- Time -->
        <div class="border-b border-base-content/10">
          <div class="flex items-center gap-3 px-4 py-3">
            <ClockIcon class="size-4 opacity-60" />
            <span class="flex-1 text-sm">Time</span>
            <button data-testid="reminder-toggle-time" class="btn btn-ghost btn-xs btn-square" @click="toggleTime">
              <span class="sr-only">Toggle time</span>
              <XMarkIcon v-if="showTime" class="size-4" />
              <PlusIcon v-else class="size-4" />
            </button>
          </div>
          <div v-if="showTime" class="flex items-center gap-2 px-4 pb-3">
            <input
              class="input input-bordered input-sm flex-1"
              data-testid="reminder-hour"
              type="number"
              min="0"
              max="23"
              :value="timeHour"
              @input="normalizeHourInput"
            />
            <span class="font-semibold opacity-50">:</span>
            <input
              class="input input-bordered input-sm flex-1"
              data-testid="reminder-minute"
              type="number"
              min="0"
              max="59"
              :value="timeMinute"
              @input="normalizeMinuteInput"
            />
          </div>
        </div>

        <!-- Actions -->
        <div class="flex gap-3 px-4 py-4">
            <button data-testid="reminder-clear" class="btn flex-1 btn-error btn-outline" @click="onClear">Clear</button>
            <button data-testid="reminder-done" class="btn flex-1 btn-primary" @click="onDone">Done</button>
          </div>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.reminder-panel {
  left: max(0.75rem, env(safe-area-inset-left));
  right: max(0.75rem, env(safe-area-inset-right));
  bottom: max(0.75rem, env(safe-area-inset-bottom));
  max-width: 320px;
  max-height: min(88vh, 680px);
  margin: 0 auto;
  overflow: auto;
  color: var(--fg-1);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
}

.reminder-search {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin: 0.75rem 0.75rem 0;
  padding: 0.625rem 0.75rem;
  color: var(--fg-1);
  border: 1.5px solid var(--color-primary);
  border-radius: 10px;
}

.reminder-search input {
  min-width: 0;
  flex: 1;
  color: inherit;
  font: inherit;
  background: transparent;
  border: 0;
  outline: none;
}

.reminder-search input::placeholder {
  color: var(--fg-4);
  opacity: 1;
}

.reminder-month-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  color: var(--fg-3);
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 8px;
}

.reminder-month-button:hover {
  color: var(--fg-1);
  background: var(--color-base-200);
}
</style>
