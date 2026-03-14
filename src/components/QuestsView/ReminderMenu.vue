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
  ChevronRightIcon,
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
const showCalendar = ref(false);
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
  return d.toISOString().slice(0, 10);
}

function todayDate() { return toDateStr(new Date()); }
function tomorrowDate() { const d = new Date(); d.setDate(d.getDate() + 1); return toDateStr(d); }
function nextWeekDate() {
  const d = new Date();
  const diff = d.getDay() === 0 ? 1 : 8 - d.getDay();
  d.setDate(d.getDate() + diff);
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
  showCalendar.value = false;
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

const timeHour = computed({
  get: () => localDueTime.value ? parseInt(localDueTime.value.slice(0, 2)) : 9,
  set: (h: number) => {
    const m = localDueTime.value ? localDueTime.value.slice(3, 5) : "00";
    localDueTime.value = `${String(h).padStart(2, "0")}:${m}`;
  },
});

const timeMinute = computed({
  get: () => localDueTime.value ? parseInt(localDueTime.value.slice(3, 5)) : 0,
  set: (m: number) => {
    const h = localDueTime.value ? localDueTime.value.slice(0, 2) : "09";
    localDueTime.value = `${h}:${String(m).padStart(2, "0")}`;
  },
});

const HOURS = Array.from({ length: 24 }, (_, i) => i);
const MINUTES = [0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55];

function toggleTime() {
  if (showTime.value) {
    showTime.value = false;
    localDueTime.value = null;
  } else {
    showTime.value = true;
    if (!localDueTime.value) localDueTime.value = "09:00";
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
    <div class="fixed inset-0 z-[199] bg-black/40 pointer-events-none" />

    <!-- Sheet -->
    <div
      ref="sheetRef"
      class="fixed bottom-0 left-0 right-0 z-[200] bg-base-200 rounded-t-2xl shadow-xl flex flex-col"
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

      <!-- ── Calendar sub-view ── -->
      <template v-else-if="showCalendar">
        <div class="flex items-center gap-2 px-4 py-3 border-b border-base-content/10">
          <button class="btn btn-ghost btn-xs btn-square" @click="showCalendar = false">
            <ChevronLeftIcon class="size-4" />
          </button>
          <span class="font-semibold flex-1">Choose a date</span>
        </div>

        <!-- Month nav -->
        <div class="flex items-center justify-between px-4 py-2">
          <button class="btn btn-ghost btn-xs btn-square" @click="prevMonth">
            <ChevronDoubleLeftIcon class="size-4" />
          </button>
          <span class="text-sm font-medium">{{ MONTH_NAMES[calMonth] }} {{ calYear }}</span>
          <button class="btn btn-ghost btn-xs btn-square" @click="nextMonth">
            <ChevronDoubleRightIcon class="size-4" />
          </button>
        </div>

        <!-- Day labels -->
        <div class="grid grid-cols-7 px-4 pb-1">
          <span
            v-for="label in DAY_LABELS" :key="label"
            class="text-center text-xs opacity-40 py-1"
          >{{ label }}</span>
        </div>

        <!-- Day cells -->
        <div class="grid grid-cols-7 px-4 pb-4 gap-y-1">
          <button
            v-for="(day, i) in calDays" :key="i"
            :disabled="!day"
            class="h-9 w-full rounded-lg text-sm flex items-center justify-center disabled:opacity-0 transition-colors"
            :class="day ? [
              localDue === calDateStr(day) ? 'bg-primary text-primary-content font-semibold' :
              isToday(day) ? 'font-semibold ring-1 ring-primary/40 hover:bg-base-300' :
              'hover:bg-base-300'
            ] : []"
            @click="day && pickDay(day)"
          >{{ day }}</button>
        </div>
      </template>

      <!-- ── Main sheet ── -->
      <template v-else>
        <!-- Quick picks -->
        <div class="flex gap-2 px-4 py-3 border-b border-base-content/10 flex-wrap">
          <button
            class="btn btn-sm rounded-full gap-1"
            :class="localDue === todayDate() ? 'btn-primary' : 'btn-ghost bg-base-300'"
            @click="localDue = todayDate()"
          >
            <StarIcon class="size-3" /> Today
          </button>
          <button
            class="btn btn-sm rounded-full gap-1"
            :class="localDue === tomorrowDate() ? 'btn-primary' : 'btn-ghost bg-base-300'"
            @click="localDue = tomorrowDate()"
          >
            <CalendarDaysIcon class="size-3" /> Tomorrow
          </button>
          <button
            class="btn btn-sm rounded-full gap-1"
            :class="localDue === nextWeekDate() ? 'btn-primary' : 'btn-ghost bg-base-300'"
            @click="localDue = nextWeekDate()"
          >
            <CalendarDaysIcon class="size-3" /> Next week
          </button>
        </div>

        <!-- Choose a date -->
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
          <button class="flex-1 flex items-center justify-between py-1 text-sm opacity-60 hover:opacity-100 transition-opacity" @click="showCalendar = true">
            <span>{{ localDue ? "Change" : "Choose a date" }}</span>
            <ChevronRightIcon class="size-4" />
          </button>
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
            <button class="btn btn-ghost btn-xs btn-square" @click="toggleTime">
              <XMarkIcon v-if="showTime" class="size-4" />
              <PlusIcon v-else class="size-4" />
            </button>
          </div>
          <div v-if="showTime" class="flex items-center gap-2 px-4 pb-3">
            <select class="select select-bordered select-sm flex-1" v-model="timeHour">
              <option v-for="h in HOURS" :key="h" :value="h">
                {{ String(h).padStart(2, "0") }}
              </option>
            </select>
            <span class="font-semibold opacity-50">:</span>
            <select class="select select-bordered select-sm flex-1" v-model="timeMinute">
              <option v-for="m in MINUTES" :key="m" :value="m">
                {{ String(m).padStart(2, "0") }}
              </option>
            </select>
          </div>
        </div>

        <!-- Actions -->
        <div class="flex gap-3 px-4 py-4">
          <button class="btn flex-1 btn-error btn-outline" @click="onClear">Clear</button>
          <button class="btn flex-1 btn-primary" @click="onDone">Done</button>
        </div>
      </template>
    </div>
  </Teleport>
</template>
