<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import type { Quest } from "../../stores/quest";
import {
  StarIcon,
  CalendarDaysIcon,
  MagnifyingGlassIcon,
  ArrowPathIcon,
  ClockIcon,
  XMarkIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  ChevronUpIcon,
  ChevronDownIcon,
} from "@heroicons/vue/24/outline";

const props = defineProps<{ quest: Quest }>();
const emit = defineEmits<{
  close: [];
  save: [payload: { due: string | null; due_time: string | null; repeat_rule: string | null }];
}>();

// ── Local state ────────────────────────────────────────────────────────────────

const localDue = ref<string | null>(props.quest.due ?? null);
const localDueTime = ref<string | null>(props.quest.due_time ?? null);

const showTime = ref(!!props.quest.due_time);

// ── Close behaviour ────────────────────────────────────────────────────────────

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    emit("close");
  }
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
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

// ── Inline calendar ────────────────────────────────────────────────────────────

interface CalCell {
  day: number;
  month: number;
  year: number;
  adjacent: boolean;
}

const today = new Date();
const calYear = ref(
  localDue.value ? parseInt(localDue.value.slice(0, 4)) : today.getFullYear()
);
const calMonth = ref(
  localDue.value ? parseInt(localDue.value.slice(5, 7)) - 1 : today.getMonth()
);

const MONTH_NAMES = ["January","February","March","April","May","June","July","August","September","October","November","December"];
const DAY_LABELS = ["Mo","Tu","We","Th","Fr","Sa","Su"];

const calDays = computed<CalCell[]>(() => {
  const y = calYear.value;
  const m = calMonth.value;
  // Monday-first offset: JS getDay() is Sunday-first (0-6), shift so Monday = 0.
  const firstDayIdx = (new Date(y, m, 1).getDay() + 6) % 7;
  const daysInMonth = new Date(y, m + 1, 0).getDate();
  const daysInPrevMonth = new Date(y, m, 0).getDate();
  const prevMonth = m === 0 ? 11 : m - 1;
  const prevYear = m === 0 ? y - 1 : y;
  const nextMonthIdx = m === 11 ? 0 : m + 1;
  const nextYear = m === 11 ? y + 1 : y;

  const cells: CalCell[] = [];
  for (let i = firstDayIdx - 1; i >= 0; i--) {
    cells.push({ day: daysInPrevMonth - i, month: prevMonth, year: prevYear, adjacent: true });
  }
  for (let d = 1; d <= daysInMonth; d++) {
    cells.push({ day: d, month: m, year: y, adjacent: false });
  }
  let nextDay = 1;
  while (cells.length % 7 !== 0) {
    cells.push({ day: nextDay, month: nextMonthIdx, year: nextYear, adjacent: true });
    nextDay++;
  }
  return cells;
});

function cellDateStr(cell: CalCell): string {
  const m = String(cell.month + 1).padStart(2, "0");
  const d = String(cell.day).padStart(2, "0");
  return `${cell.year}-${m}-${d}`;
}

function isToday(cell: CalCell): boolean {
  return cellDateStr(cell) === toDateStr(today);
}

function prevMonth() {
  if (calMonth.value === 0) { calMonth.value = 11; calYear.value--; }
  else calMonth.value--;
}

function nextMonth() {
  if (calMonth.value === 11) { calMonth.value = 0; calYear.value++; }
  else calMonth.value++;
}

function pickCell(cell: CalCell) {
  localDue.value = cellDateStr(cell);
  if (cell.adjacent) {
    calMonth.value = cell.month;
    calYear.value = cell.year;
  }
}

// ── Repeat ─────────────────────────────────────────────────────────────────────

type RepeatUnit = "day" | "week" | "month" | "year";
const WEEKDAY_TOKENS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] as const;
const UNIT_LABEL: Record<RepeatUnit, string> = { day: "day", week: "week", month: "month", year: "year" };

const repeatOn = ref(false);
const repeatAmount = ref(1);
const repeatUnit = ref<RepeatUnit>("day");
const repeatDays = ref<Set<string>>(new Set());

function loadRepeatState(rule: string | null) {
  if (!rule) {
    repeatOn.value = false;
    return;
  }
  try {
    const r = JSON.parse(rule);
    switch (r.preset) {
      case "daily":
        repeatAmount.value = 1; repeatUnit.value = "day"; repeatDays.value = new Set();
        break;
      case "weekly":
        repeatAmount.value = 1; repeatUnit.value = "week"; repeatDays.value = new Set();
        break;
      case "monthly":
        repeatAmount.value = 1; repeatUnit.value = "month"; repeatDays.value = new Set();
        break;
      case "yearly":
        repeatAmount.value = 1; repeatUnit.value = "year"; repeatDays.value = new Set();
        break;
      case "weekdays":
        repeatAmount.value = 1; repeatUnit.value = "day"; repeatDays.value = new Set(["Mo", "Tu", "We", "Th", "Fr"]);
        break;
      case "weekends":
        repeatAmount.value = 1; repeatUnit.value = "day"; repeatDays.value = new Set(["Sa", "Su"]);
        break;
      default: {
        const interval = Math.max(1, Math.min(99, Number(r.interval) || 1));
        const unit: RepeatUnit = (["day", "week", "month", "year"] as string[]).includes(r.unit) ? r.unit : "day";
        const days: string[] = Array.isArray(r.days_of_week)
          ? r.days_of_week.filter((d: unknown) => (WEEKDAY_TOKENS as readonly string[]).includes(d as string))
          : [];
        repeatAmount.value = interval;
        repeatUnit.value = unit;
        repeatDays.value = new Set(days);
        break;
      }
    }
    repeatOn.value = true;
  } catch {
    repeatOn.value = false;
  }
}

loadRepeatState(props.quest.repeat_rule ?? null);

const orderedRepeatDays = computed(() => WEEKDAY_TOKENS.filter((d) => repeatDays.value.has(d)));

const repeatSummary = computed<string>(() => {
  if (!repeatOn.value) return "";
  const n = repeatAmount.value;
  const days = orderedRepeatDays.value;
  const byWeekday = repeatUnit.value === "day" && days.length > 0;
  let base: string;
  if (byWeekday) {
    base = n === 1 ? "Every" : `Every ${n}`;
    base += ` · ${days.join(", ")}`;
  } else {
    base = n === 1 ? `Every ${UNIT_LABEL[repeatUnit.value]}` : `Every ${n} ${UNIT_LABEL[repeatUnit.value]}s`;
  }
  return base;
});

function toggleRepeatOn() {
  repeatOn.value = !repeatOn.value;
}

function decRepeatAmount() {
  repeatAmount.value = Math.max(1, repeatAmount.value - 1);
}

function incRepeatAmount() {
  repeatAmount.value = Math.min(99, repeatAmount.value + 1);
}

function normalizeRepeatAmountInput(event: Event) {
  const n = parseInt((event.target as HTMLInputElement).value, 10);
  repeatAmount.value = Number.isFinite(n) ? Math.min(99, Math.max(1, n)) : 1;
}

function setRepeatUnit(unit: RepeatUnit) {
  repeatUnit.value = unit;
}

function toggleRepeatDay(day: string) {
  const next = new Set(repeatDays.value);
  if (next.has(day)) next.delete(day);
  else next.add(day);
  repeatDays.value = next;
}

function serializeRepeatRule(): string | null {
  if (!repeatOn.value) return null;
  const days = orderedRepeatDays.value;
  return JSON.stringify({
    preset: "custom",
    interval: repeatAmount.value,
    unit: repeatUnit.value,
    ...(days.length ? { days_of_week: days } : {}),
  });
}

// ── Time ───────────────────────────────────────────────────────────────────────

function clampTimePart(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

function wrapTimePart(value: number, max: number): number {
  return ((value % (max + 1)) + (max + 1)) % (max + 1);
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

const timeValueLabel = computed(() =>
  showTime.value ? `${String(timeHour.value).padStart(2, "0")}:${String(timeMinute.value).padStart(2, "0")}` : "—"
);

function sanitizeDigitsInput(event: Event) {
  const el = event.target as HTMLInputElement;
  el.value = el.value.replace(/\D/g, "").slice(0, 2);
}

function normalizeHourInput(event: Event) {
  sanitizeDigitsInput(event);
  timeHour.value = (event.target as HTMLInputElement).value;
}

function normalizeMinuteInput(event: Event) {
  sanitizeDigitsInput(event);
  timeMinute.value = (event.target as HTMLInputElement).value;
}

function incHour() { timeHour.value = wrapTimePart(timeHour.value + 1, 23); }
function decHour() { timeHour.value = wrapTimePart(timeHour.value - 1, 23); }
function incMinute() { timeMinute.value = wrapTimePart(timeMinute.value + 1, 59); }
function decMinute() { timeMinute.value = wrapTimePart(timeMinute.value - 1, 59); }

function onTimeKeydown(event: KeyboardEvent, part: "hour" | "minute") {
  const input = event.target as HTMLInputElement;
  if (event.key === "Enter") {
    input.blur();
    return;
  }
  if (event.key === "ArrowUp") {
    event.preventDefault();
    if (part === "hour") incHour(); else incMinute();
    input.select();
  }
  if (event.key === "ArrowDown") {
    event.preventDefault();
    if (part === "hour") decHour(); else decMinute();
    input.select();
  }
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
    repeat_rule: serializeRepeatRule(),
  });
}
</script>

<template>
  <Teleport to="body">
    <!-- Visual backdrop -->
    <div data-testid="reminder-backdrop" class="fixed inset-0 z-[199] bg-black/30" @click="emit('close')" />

    <!-- Sheet -->
    <div
      class="reminder-panel fixed z-[200] bg-base-100 shadow-xl flex flex-col"
      style="padding-bottom: env(safe-area-inset-bottom)"
      @click.stop
    >
      <div class="reminder-panel-header">
        <label class="reminder-search">
          <MagnifyingGlassIcon class="size-4 opacity-50" />
          <input type="text" placeholder="Type a date…" />
        </label>
        <button data-testid="reminder-close" class="btn btn-ghost btn-xs btn-square" aria-label="Close reminder" @click="emit('close')">
          <XMarkIcon class="size-4" />
        </button>
      </div>

      <div class="reminder-panel-body">

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

        <!-- Calendar -->
        <div class="px-3 py-3 border-b border-base-content/10">
          <div class="mb-2 flex items-center justify-between">
            <button class="reminder-month-button" @click="prevMonth" aria-label="Previous month"><ChevronLeftIcon class="size-4" /></button>
            <span class="text-sm font-medium">{{ MONTH_NAMES[calMonth] }} {{ calYear }}</span>
            <button class="reminder-month-button" @click="nextMonth" aria-label="Next month"><ChevronRightIcon class="size-4" /></button>
          </div>
          <div class="grid grid-cols-7 pb-2">
            <span
              v-for="label in DAY_LABELS" :key="label"
              class="text-center text-xs opacity-40 py-1"
            >{{ label }}</span>
          </div>
          <div class="grid grid-cols-7 gap-y-1">
            <button
              v-for="(cell, i) in calDays" :key="i"
              class="cal-day"
              :class="{
                selected: localDue === cellDateStr(cell),
                today: isToday(cell),
                adjacent: cell.adjacent,
              }"
              @click="pickCell(cell)"
            >{{ cell.day }}</button>
          </div>
        </div>

        <!-- Time -->
        <div class="border-b border-base-content/10">
          <button
            type="button"
            data-testid="reminder-toggle-time"
            class="flex items-center w-full gap-3 px-4 py-3 hover:bg-base-300 transition-colors"
            @click="toggleTime"
          >
            <ClockIcon class="size-4 opacity-60 shrink-0" />
            <span class="flex-1 text-sm text-left">Time</span>
            <span class="text-sm" :class="showTime ? 'text-success font-semibold' : 'opacity-40'">{{ timeValueLabel }}</span>
            <ChevronDownIcon class="size-3.5 opacity-40 transition-transform shrink-0" :class="{ 'rotate-180': showTime }" />
          </button>
          <div v-if="showTime" class="rem-time-picker flex items-center justify-center gap-3 pb-3">
            <div class="rem-time-field">
              <button type="button" class="rem-time-step" aria-label="Increase hour" @click="incHour"><ChevronUpIcon class="size-3.5" /></button>
              <input
                class="rem-time-num"
                data-testid="reminder-hour"
                type="text"
                inputmode="numeric"
                maxlength="2"
                aria-label="Hour"
                :value="String(timeHour).padStart(2, '0')"
                @input="normalizeHourInput"
                @keydown="onTimeKeydown($event, 'hour')"
                @focus="($event.target as HTMLInputElement).select()"
              />
              <button type="button" class="rem-time-step" aria-label="Decrease hour" @click="decHour"><ChevronDownIcon class="size-3.5" /></button>
            </div>
            <span class="text-lg font-bold opacity-50">:</span>
            <div class="rem-time-field">
              <button type="button" class="rem-time-step" aria-label="Increase minute" @click="incMinute"><ChevronUpIcon class="size-3.5" /></button>
              <input
                class="rem-time-num"
                data-testid="reminder-minute"
                type="text"
                inputmode="numeric"
                maxlength="2"
                aria-label="Minute"
                :value="String(timeMinute).padStart(2, '0')"
                @input="normalizeMinuteInput"
                @keydown="onTimeKeydown($event, 'minute')"
                @focus="($event.target as HTMLInputElement).select()"
              />
              <button type="button" class="rem-time-step" aria-label="Decrease minute" @click="decMinute"><ChevronDownIcon class="size-3.5" /></button>
            </div>
          </div>
        </div>

        <!-- Repeat -->
        <div class="border-b border-base-content/10">
          <div class="flex items-center gap-3 px-4 py-3">
            <ArrowPathIcon class="size-4 opacity-60 shrink-0" />
            <span class="flex-1 text-sm">Repeat</span>
            <span class="text-sm" :class="repeatOn ? 'text-success font-semibold' : 'opacity-40'">{{ repeatSummary || "Off" }}</span>
            <button
              type="button"
              class="rem-switch"
              role="switch"
              :aria-checked="repeatOn"
              aria-label="Toggle repeat"
              data-testid="reminder-repeat-toggle"
              @click="toggleRepeatOn"
            >
              <span class="rem-switch-knob" />
            </button>
          </div>
          <div v-if="repeatOn" class="px-4 pb-3 flex flex-col gap-2.5">
            <div class="flex items-center gap-2 flex-wrap">
              <span class="text-sm opacity-60 shrink-0">Every</span>
              <div class="rem-amount">
                <button type="button" class="rem-amt-btn" aria-label="Decrease" @click="decRepeatAmount">−</button>
                <input
                  type="number"
                  class="rem-amt-input"
                  min="1"
                  max="99"
                  inputmode="numeric"
                  :value="repeatAmount"
                  @input="normalizeRepeatAmountInput"
                />
                <button type="button" class="rem-amt-btn" aria-label="Increase" @click="incRepeatAmount">+</button>
              </div>
              <div class="rem-unit">
                <button type="button" :aria-pressed="repeatUnit === 'day'" @click="setRepeatUnit('day')">Day</button>
                <button type="button" :aria-pressed="repeatUnit === 'week'" @click="setRepeatUnit('week')">Week</button>
                <button type="button" :aria-pressed="repeatUnit === 'month'" @click="setRepeatUnit('month')">Month</button>
                <button type="button" :aria-pressed="repeatUnit === 'year'" @click="setRepeatUnit('year')">Year</button>
              </div>
            </div>
            <div v-if="repeatUnit === 'day'" class="rem-weekdays">
              <button
                v-for="wd in WEEKDAY_TOKENS" :key="wd"
                type="button"
                :aria-pressed="repeatDays.has(wd)"
                @click="toggleRepeatDay(wd)"
              >{{ wd[0] }}</button>
            </div>
          </div>
        </div>
      </div>

      <!-- Actions -->
      <div class="reminder-panel-actions flex gap-3 px-4 py-4">
        <button data-testid="reminder-clear" class="btn flex-1 btn-error btn-outline" @click="onClear">Clear</button>
        <button data-testid="reminder-done" class="btn flex-1 btn-primary" @click="onDone">Done</button>
      </div>
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
  overflow: hidden;
  color: var(--fg-1);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
}

.reminder-panel-header {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 0.5rem;
  padding: 0.75rem 0.75rem 0;
}

.reminder-panel-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  scrollbar-width: thin;
}

.reminder-panel-actions {
  flex-shrink: 0;
  background: var(--color-base-100);
  border-top: 1px solid var(--color-border-soft);
}

.reminder-search {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 0.5rem;
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

/* Calendar day cells: circular, ring on today, filled on selected. */
.cal-day {
  aspect-ratio: 1;
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  color: var(--fg-1);
  font-size: 0.8125rem;
  font-weight: 600;
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 9999px;
  transition: background-color 120ms;
}

.cal-day:hover {
  background: var(--color-base-200);
}

.cal-day.adjacent {
  color: var(--fg-5);
  font-weight: 500;
}

.cal-day.today {
  color: var(--color-success);
  font-weight: 700;
  box-shadow: inset 0 0 0 1.5px var(--color-success);
}

.cal-day.selected {
  color: var(--color-primary-content);
  background: var(--color-primary);
  box-shadow: none;
}

/* Time hour/minute steppers. */
.rem-time-field {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  width: 52px;
  padding: 4px 6px;
  background: var(--color-base-200);
  border: 1px solid var(--color-border-soft);
  border-radius: 10px;
}

.rem-time-step {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  padding: 2px;
  color: var(--fg-4);
  background: transparent;
  border: 0;
  border-radius: 6px;
  cursor: pointer;
}

.rem-time-step:hover {
  color: var(--fg-1);
  background: var(--color-base-300);
}

.rem-time-num {
  width: 100%;
  padding: 2px 0;
  color: var(--fg-1);
  font-size: 22px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  text-align: center;
  background: transparent;
  border: 0;
  outline: none;
}

.rem-time-num:focus {
  color: var(--color-primary);
}

/* Repeat on/off switch. */
.rem-switch {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  width: 34px;
  height: 20px;
  padding: 2px;
  background: var(--color-base-300);
  border: 0;
  border-radius: 10px;
  cursor: pointer;
}

.rem-switch-knob {
  width: 16px;
  height: 16px;
  background: var(--color-base-100);
  border-radius: 50%;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
  transition: transform 140ms;
}

.rem-switch[aria-checked="true"] {
  background: var(--color-primary);
}

.rem-switch[aria-checked="true"] .rem-switch-knob {
  transform: translateX(14px);
}

/* Repeat amount stepper + unit segmented control. */
.rem-amount {
  display: inline-flex;
  align-items: center;
  overflow: hidden;
  border: 1px solid var(--color-border-soft);
  border-radius: 8px;
}

.rem-amt-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 30px;
  color: var(--fg-2);
  font-size: 15px;
  font-weight: 600;
  background: var(--color-base-200);
  border: 0;
  cursor: pointer;
}

.rem-amt-btn:hover {
  background: var(--color-base-300);
}

.rem-amt-input {
  width: 32px;
  height: 30px;
  color: var(--fg-1);
  font-size: 14px;
  font-weight: 600;
  text-align: center;
  background: var(--color-base-100);
  border: 0;
  border-left: 1px solid var(--color-border-soft);
  border-right: 1px solid var(--color-border-soft);
  outline: none;
}

.rem-amt-input::-webkit-outer-spin-button,
.rem-amt-input::-webkit-inner-spin-button {
  margin: 0;
  -webkit-appearance: none;
}

.rem-unit {
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  background: var(--color-base-200);
  border-radius: 8px;
}

.rem-unit button {
  padding: 7px 9px;
  color: var(--fg-3);
  font-size: 12px;
  font-weight: 600;
  background: transparent;
  border: 0;
  border-radius: 6px;
  cursor: pointer;
}

.rem-unit button:hover {
  color: var(--fg-1);
}

.rem-unit button[aria-pressed="true"] {
  color: var(--fg-1);
  background: var(--color-base-100);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
}

/* Repeat weekday picker. */
.rem-weekdays {
  display: flex;
  gap: 4px;
}

.rem-weekdays button {
  flex: 1;
  aspect-ratio: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  color: var(--fg-3);
  font-size: 12px;
  font-weight: 600;
  background: transparent;
  border: 1px solid var(--color-border-soft);
  border-radius: 50%;
  cursor: pointer;
}

.rem-weekdays button:hover {
  background: var(--color-base-200);
}

.rem-weekdays button[aria-pressed="true"] {
  color: var(--color-primary-content);
  background: var(--color-primary);
  border-color: var(--color-primary);
}
</style>
