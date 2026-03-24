<script setup lang="ts">
import { ref } from "vue";
import { useQuestStore, type Quest } from "../../stores/quest";
import {
  PaperClipIcon,
  TagIcon,
  FlagIcon,
  ClockIcon,
  BoltIcon,
  EllipsisVerticalIcon,
  ChevronUpIcon,
  ExclamationCircleIcon,
} from "@heroicons/vue/24/outline";
import ReminderMenu from "./ReminderMenu.vue";

defineProps<{ quests: Quest[] }>();
const store = useQuestStore();

// ── Expand / collapse ─────────────────────────────────────────────────────────

const expandedId = ref<string | null>(null);

function toggle(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
}

// ── Active actions ────────────────────────────────────────────────────────────

async function completeQuest(id: string) {
  await store.updateQuest(id, { status: "completed" });
}

async function setMain(quest: Quest) {
  await store.setMainQuest(quest.id);
}

async function onTitleBlur(quest: Quest, e: Event) {
  const val = (e.target as HTMLElement).innerText.trim();
  if (val && val !== quest.title) await store.updateQuest(quest.id, { title: val });
  else if (!val) (e.target as HTMLElement).innerText = quest.title;
}

function onTitleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter") { e.preventDefault(); (e.target as HTMLElement).blur(); }
}

async function onDescBlur(quest: Quest, e: Event) {
  const val = (e.target as HTMLTextAreaElement).value.trim() || null;
  if (val !== quest.description) await store.updateQuest(quest.id, { description: val });
}

// ── History actions ───────────────────────────────────────────────────────────

async function restore(id: string) {
  await store.updateQuest(id, { status: "active" });
}

// ── Priority ──────────────────────────────────────────────────────────────────

const PRIORITIES = [1, 2, 3, 4] as const;
const PRIORITY_LABELS: Record<number, string> = { 1: "None", 2: "Low", 3: "Medium", 4: "Urgent" };
const PRIORITY_COLORS: Record<number, string> = { 1: "oklch(var(--color-base-content)/0.3)", 2: "oklch(var(--color-success))", 3: "oklch(var(--color-warning))", 4: "oklch(var(--color-error))" };

async function cyclePriority(quest: Quest) {
  const idx = PRIORITIES.indexOf(quest.priority as typeof PRIORITIES[number]);
  const next = PRIORITIES[(idx + 1) % PRIORITIES.length];
  await store.updateQuest(quest.id, { priority: next });
}

// ── Energy ────────────────────────────────────────────────────────────────────

const ENERGIES = ["low", "medium", "high"] as const;
const ENERGY_COLORS: Record<string, string> = { low: "oklch(var(--color-success))", medium: "oklch(var(--color-warning))", high: "oklch(var(--color-error))" };

async function cycleEnergy(quest: Quest) {
  const idx = ENERGIES.indexOf(quest.energy as typeof ENERGIES[number]);
  const next = ENERGIES[(idx + 1) % ENERGIES.length];
  await store.updateQuest(quest.id, { energy: next });
}

// ── Reminder menu ─────────────────────────────────────────────────────────────

const reminderQuestId = ref<string | null>(null);

async function onReminderSave(
  quest: Quest,
  payload: { due: string | null; due_time: string | null; repeat_rule: string | null }
) {
  await store.updateQuest(quest.id, payload);
  reminderQuestId.value = null;
}

// ── More menu ─────────────────────────────────────────────────────────────────

const moreMenu = ref<{ right: number; y: number; quest: Quest } | null>(null);

function openMore(e: MouseEvent, quest: Quest) {
  e.stopPropagation();
  moreMenu.value = { right: window.innerWidth - e.clientX, y: e.clientY, quest };
  window.addEventListener("click", closeMore, { once: true });
}

function closeMore() { moreMenu.value = null; }

async function menuAction1() {
  if (!moreMenu.value) return;
  const quest = moreMenu.value.quest;
  if (quest.status !== "active") {
    await restore(quest.id);
  } else {
    await store.updateQuest(quest.id, { status: "abandoned" });
    expandedId.value = null;
  }
  closeMore();
}

async function menuDelete() {
  if (!moreMenu.value) return;
  await store.deleteQuest(moreMenu.value.quest.id);
  expandedId.value = null;
  closeMore();
}

// ── Metadata ──────────────────────────────────────────────────────────────────

function formatDue(due: string): string {
  const date = new Date(due + "T00:00:00");
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function formatRepeat(repeatRule: string): string {
  try {
    const rule = JSON.parse(repeatRule);
    const preset = rule.preset;
    const labels: Record<string, string> = {
      daily: "every day", weekdays: "weekdays", weekends: "weekends",
      weekly: "every week", monthly: "every month", yearly: "every year",
    };
    if (preset && preset !== "custom" && preset !== "none") return labels[preset] ?? preset;
    if (!preset || preset === "none") return "";
    const n = rule.interval ?? 1;
    const unit = rule.unit ?? "week";
    const days = (rule.days_of_week as string[] | undefined)?.join(",") ?? "";
    return `every ${n} ${unit}${n > 1 ? "s" : ""}${days ? ` (${days})` : ""}`;
  } catch { return ""; }
}

function formatTime(time: string): string {
  const [h, m] = time.split(":").map(Number);
  return `${h}:${String(m).padStart(2, "0")}`;
}

function pillText(quest: Quest): string {
  const parts: string[] = [];
  if (quest.due) {
    const date = formatDue(quest.due);
    const time = quest.due_time ? ` at ${formatTime(quest.due_time)}` : "";
    parts.push(date + time);
  }
  if (quest.repeat_rule) {
    const r = formatRepeat(quest.repeat_rule);
    if (r) parts.push(r);
  }
  return parts.join(", ");
}

function formatTimestamp(quest: Quest): string {
  const raw = quest.completed_at ?? quest.updated_at;
  const date = new Date(raw);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const time = date.toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", hour12: false });
  if (date.toDateString() === today.toDateString()) return `Today, ${time}`;
  if (date.toDateString() === yesterday.toDateString()) return `Yesterday, ${time}`;
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" }) + `, ${time}`;
}
</script>

<template>
  <ul class="flex flex-col gap-1">
    <li
      v-for="quest in quests"
      :key="quest.id"
      class="quest-row"
    >

      <!-- Collapsed row -->
      <div
        v-if="expandedId !== quest.id"
        class="quest-row-surface flex items-center gap-3 px-4 py-3 rounded-xl cursor-pointer hover:bg-base-200 transition-colors select-none"
        @click="toggle(quest.id)"
      >
        <!-- History: checked checkbox restores; Active: unchecked completes -->
        <input
          v-if="quest.status !== 'active'"
          type="checkbox"
          class="checkbox checkbox-sm shrink-0 cursor-pointer"
          :class="quest.status === 'completed' ? 'checkbox-success' : 'checkbox-warning'"
          checked
          @click.prevent.stop="restore(quest.id)"
        />
        <input v-else type="checkbox" class="checkbox checkbox-sm" :checked="false" @change.stop="completeQuest(quest.id)" @click.stop />

        <span
          v-if="quest.status !== 'active'"
          class="badge badge-sm shrink-0 font-mono"
          :class="quest.status === 'completed' ? 'badge-success' : 'badge-warning'"
        >{{ formatTimestamp(quest) }}</span>

        <span class="flex-1 text-sm" :class="{ 'line-through opacity-50': quest.status !== 'active' }">{{ quest.title }}</span>
      </div>

      <!-- Expanded card -->
      <div v-else class="card card-bordered bg-base-200 shadow-sm">
        <div class="card-body px-4 py-4 gap-4">

          <!-- Header -->
          <div class="flex items-center gap-2">
            <input
              v-if="quest.status !== 'active'"
              type="checkbox"
              class="checkbox checkbox-sm shrink-0 cursor-pointer"
              :class="quest.status === 'completed' ? 'checkbox-success' : 'checkbox-warning'"
              checked
              @click.prevent.stop="restore(quest.id)"
            />
            <input v-else type="checkbox" class="checkbox checkbox-sm" :checked="false" @change.stop="completeQuest(quest.id)" @click.stop />

            <span
              v-if="quest.status !== 'active'"
              class="flex-1 font-semibold text-base line-through opacity-60"
            >{{ quest.title }}</span>
            <span
              v-else
              class="flex-1 font-semibold text-base outline-none cursor-text"
              contenteditable="true"
              @blur="onTitleBlur(quest, $event)"
              @keydown="onTitleKeydown"
            >{{ quest.title }}</span>

            <span
              v-if="quest.status !== 'active'"
              class="badge badge-sm font-mono"
              :class="quest.status === 'completed' ? 'badge-success' : 'badge-warning'"
            >{{ formatTimestamp(quest) }}</span>
            <button
              v-else
              class="btn btn-ghost btn-xs btn-square"
              :class="store.activeQuest?.id === quest.id ? '' : 'opacity-30'"
              style="color: oklch(0.85 0.15 85)"
              @click.stop="setMain(quest)"
              title="Set Main"
            >
              <ExclamationCircleIcon class="size-5" />
            </button>

            <button class="btn btn-ghost btn-xs btn-square" @click.stop="toggle(quest.id)" title="Collapse">
              <ChevronUpIcon class="size-4" />
            </button>
          </div>

          <!-- Description -->
          <textarea
            v-if="quest.status === 'active'"
            class="textarea textarea-ghost w-full text-sm opacity-60 resize-none p-0 min-h-16"
            :value="quest.description ?? ''"
            placeholder="Description"
            rows="3"
            @blur="onDescBlur(quest, $event)"
          />
          <p v-else-if="quest.description" class="text-sm opacity-60">{{ quest.description }}</p>

          <!-- Footer -->
          <div class="flex items-center justify-between mt-1">
            <button
              v-if="quest.status === 'active'"
              class="flex items-center gap-1 text-xs opacity-50 hover:opacity-80 transition-opacity"
              @click.stop="reminderQuestId = quest.id"
              title="Reminder"
            >
              <ClockIcon class="size-4" />
              <span v-if="pillText(quest)">{{ pillText(quest) }}</span>
            </button>
            <div v-else />

            <div class="flex items-center">
              <template v-if="quest.status === 'active'">
                <button class="btn btn-ghost btn-xs btn-square opacity-25" disabled title="Attachment">
                  <PaperClipIcon class="size-4" />
                </button>
                <button class="btn btn-ghost btn-xs btn-square opacity-25" disabled title="Label">
                  <TagIcon class="size-4" />
                </button>
                <button
                  class="btn btn-ghost btn-xs btn-square"
                  :style="{ color: PRIORITY_COLORS[quest.priority] }"
                  @click.stop="cyclePriority(quest)"
                  :title="PRIORITY_LABELS[quest.priority]"
                >
                  <FlagIcon class="size-4" />
                </button>
                <button
                  class="btn btn-ghost btn-xs btn-square"
                  :style="{ color: ENERGY_COLORS[quest.energy] }"
                  @click.stop="cycleEnergy(quest)"
                  :title="`Energy: ${quest.energy}`"
                >
                  <BoltIcon class="size-4" />
                </button>
              </template>
              <button class="btn btn-ghost btn-xs btn-square" @click.stop="openMore($event, quest)" title="More">
                <EllipsisVerticalIcon class="size-4" />
              </button>
            </div>
          </div>

        </div>
      </div>

    </li>
  </ul>

  <!-- Reminder menu (active only) -->
  <ReminderMenu
    v-if="reminderQuestId !== null"
    :quest="quests.find(q => q.id === reminderQuestId)!"
    @close="reminderQuestId = null"
    @save="onReminderSave(quests.find(q => q.id === reminderQuestId)!, $event)"
  />

  <!-- More menu -->
  <Teleport to="body">
    <ul
      v-if="moreMenu"
      class="menu bg-base-200 rounded-box shadow-lg w-36 fixed z-50"
      :style="{ top: `${moreMenu.y}px`, right: `${moreMenu.right}px` }"
      @click.stop
    >
      <li><a @click="menuAction1">{{ moreMenu?.quest.status !== 'active' ? 'Make active' : 'Abandon' }}</a></li>
      <li><a class="text-error" @click="menuDelete">Delete</a></li>
    </ul>
  </Teleport>
</template>

<style scoped>
.quest-row {
  list-style: none;
}
</style>
