<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useQuestStore, type Quest, type UpdateQuestInput } from "../../stores/quest";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../../stores/space";
import { useContextMenu } from "../../composables/useContextMenu";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import { ArrowPathIcon } from "@heroicons/vue/24/outline";
import ReminderMenu from "./ReminderMenu.vue";
import QuestEditor from "../QuestEditor.vue";

defineProps<{ quests: Quest[] }>();
const store = useQuestStore();
const { t } = useI18n();
const spaceStore = useSpaceStore();
const contextMenu = useContextMenu();
const { ensureReminderNotificationsAllowed } = useReminderNotifications();

function spaceName(quest: Quest): string {
  return spaceStore.spaces.find((s) => s.id === quest.space_id)?.name ?? "";
}

function spaceCss(quest: Quest): string {
  return SPACE_COLOR_CLASS[quest.space_id] ?? "";
}

function statusLabel(quest: Quest): string {
  return quest.status === "completed" ? "Completed" : "Abandoned";
}

function onContextMenu(e: MouseEvent, quest: Quest) {
  const moveItems = spaceStore.spaces
    .filter((s) => s.id !== quest.space_id)
    .map((s) => ({
      label: s.name,
      action: () => store.updateQuest(quest.id, { space_id: s.id }),
    }));
  const items = [];
  if (quest.status === "active") {
    items.push({ label: "Complete", action: () => store.updateQuest(quest.id, { status: "completed" }) });
    items.push({ separator: true });
    items.push({ label: "Set Focus", action: () => store.setFocusQuest(quest.id) });
  }
  if (quest.status !== "active") {
    items.push({ label: "Make active", action: () => store.updateQuest(quest.id, { status: "active" }) });
  }
  if (moveItems.length > 0) {
    items.push({ label: "Move to space", children: moveItems });
  } else {
    items.push({ label: "Move to space", disabled: true });
  }
  items.push({ separator: true });
  if (quest.status === "active") {
    items.push({ label: "Abandon", action: () => store.updateQuest(quest.id, { status: "abandoned" }) });
  }
  items.push({ label: "Delete", action: () => store.deleteQuest(quest.id) });
  contextMenu.open(e, items);
}

// ── Expand / collapse ─────────────────────────────────────────────────────────

const expandedId = ref<string | null>(null);

function toggle(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
}

// ── Active actions ────────────────────────────────────────────────────────────

async function completeQuest(id: string) {
  await store.updateQuest(id, { status: "completed" });
}

async function setFocus(quest: Quest) {
  await store.setFocusQuest(quest.id);
}

async function updateQuest(quest: Quest, patch: UpdateQuestInput) {
  await store.updateQuest(quest.id, patch);
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

// ── Reminder menu ─────────────────────────────────────────────────────────────

const reminderQuestId = ref<string | null>(null);

async function onReminderSave(
  quest: Quest,
  payload: { due: string | null; due_time: string | null; repeat_rule: string | null }
) {
  if (!(await ensureReminderNotificationsAllowed(payload))) {
    return;
  }
  await store.updateQuest(quest.id, payload);
  reminderQuestId.value = null;
}

function openMore(e: MouseEvent, quest: Quest) {
  onContextMenu(e, quest);
}

// ── Metadata ──────────────────────────────────────────────────────────────────

function formatDue(due: string): string {
  const date = new Date(due + "T00:00:00");
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function localDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function smartDueLabel(quest: Quest): string {
  if (!quest.due) return "";
  const now = new Date();
  const todayStr = localDateStr(now);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const tomorrowStr = localDateStr(tomorrow);
  const time = quest.due_time ? `, ${formatTime(quest.due_time)}` : "";
  if (quest.due === todayStr) return t("quest.today") + time;
  if (quest.due === tomorrowStr) return t("quest.tomorrow") + time;
  return formatDue(quest.due) + time;
}

function dueBadgeClass(quest: Quest): string {
  if (!quest.due) return "";
  const todayStr = localDateStr(new Date());
  if (quest.due < todayStr) return "badge-error";
  if (quest.due === todayStr) return "badge-success";
  return "badge-ghost";
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
        class="quest-row-surface"
        @click="toggle(quest.id)"
        @contextmenu="onContextMenu($event, quest)"
      >
        <!-- History: status glyph restores; Active: empty square completes -->
        <button
          v-if="quest.status !== 'active'"
          class="quest-check"
          :class="quest.status"
          :aria-label="`Make ${quest.title} active`"
          @click.prevent.stop="restore(quest.id)"
        />
        <button v-else class="quest-check" :aria-label="`Complete ${quest.title}`" @click.stop="completeQuest(quest.id)" />

        <span
          v-if="quest.status !== 'active'"
          class="quest-status-badge"
          :class="quest.status"
        >{{ statusLabel(quest) }} · {{ formatTimestamp(quest) }}</span>

        <span v-if="quest.due && quest.status === 'active'" class="quest-due-badge" :class="dueBadgeClass(quest)">
          {{ smartDueLabel(quest) }}
          <ArrowPathIcon v-if="quest.repeat_rule" class="size-3.5" />
        </span>
        <span v-else-if="quest.repeat_rule && quest.status === 'active'" class="quest-repeat-badge">
          <ArrowPathIcon class="size-3.5" />
          {{ formatRepeat(quest.repeat_rule) }}
        </span>
        <span class="quest-title" :class="quest.status !== 'active' ? quest.status : ''">{{ quest.title }}</span>
        <span class="quest-space-badge badge badge-xs" :class="spaceCss(quest)">{{ spaceName(quest) }}</span>
      </div>

      <!-- Expanded list item: same editor used by the active-card expanded state. -->
      <QuestEditor
        v-else
        :quest="quest"
        :space-name="spaceName(quest)"
        :is-focus="store.activeQuest?.id === quest.id"
        :priority-color="PRIORITY_COLORS[quest.priority]"
        :priority-label="PRIORITY_LABELS[quest.priority]"
        :reminder-text="pillText(quest)"
        :timestamp-text="quest.status !== 'active' ? formatTimestamp(quest) : ''"
        @contextmenu="onContextMenu($event, quest)"
        @update="updateQuest(quest, $event)"
        @complete="completeQuest(quest.id)"
        @restore="restore(quest.id)"
        @set-focus="setFocus(quest)"
        @collapse="toggle(quest.id)"
        @open-reminder="reminderQuestId = quest.id"
        @cycle-priority="cyclePriority(quest)"
        @more="openMore($event, quest)"
      />

    </li>
  </ul>

  <!-- Reminder menu (active only) -->
  <ReminderMenu
    v-if="reminderQuestId !== null"
    :quest="quests.find(q => q.id === reminderQuestId)!"
    @close="reminderQuestId = null"
    @save="onReminderSave(quests.find(q => q.id === reminderQuestId)!, $event)"
  />
</template>

<style scoped>
.quest-row {
  list-style: none;
}

.quest-row-surface {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4375rem 0.75rem;
  color: var(--fg-1);
  cursor: pointer;
  user-select: none;
  border-radius: 6px;
  transition: background-color var(--dur-normal), color var(--dur-normal);
}

.quest-row-surface:hover { background: var(--color-base-200); }

.quest-check {
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

.quest-check:hover { border-color: var(--fg-3); }
.quest-check.completed { background: var(--color-success); border-color: var(--color-success); }
.quest-check.completed::before,
.quest-check.completed::after {
  content: "";
  position: absolute;
  background: #fff;
  border-radius: 2px;
  transform-origin: left center;
}
.quest-check.completed::before { left: 3px; top: 9px; width: 4px; height: 2px; transform: rotate(45deg); }
.quest-check.completed::after { left: 5.5px; top: 10.5px; width: 9px; height: 2px; transform: rotate(-45deg); }
.quest-check.abandoned { border-color: var(--fg-5); }
.quest-check.abandoned::before,
.quest-check.abandoned::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  width: 11px;
  height: 1.8px;
  background: var(--fg-4);
  border-radius: 2px;
}
.quest-check.abandoned::before { transform: translate(-50%, -50%) rotate(45deg); }
.quest-check.abandoned::after { transform: translate(-50%, -50%) rotate(-45deg); }

.quest-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--fg-1);
  font-size: 0.875rem;
  line-height: 1.4;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.quest-title.completed {
  color: var(--fg-4);
  text-decoration: line-through;
}

.quest-title.abandoned { color: var(--fg-4); }

.quest-status-badge,
.quest-due-badge,
.quest-repeat-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  flex-shrink: 0;
  padding: 0.125rem 0.5rem;
  font-size: 0.6875rem;
  font-weight: 500;
  line-height: 1.4;
  border-radius: 6px;
}

.quest-status-badge {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.quest-status-badge.completed {
  color: #fff;
  background: var(--color-success);
}

.quest-status-badge.abandoned {
  color: var(--fg-4);
  background: transparent;
  border: 1px solid var(--color-border-soft);
}

.quest-due-badge.badge-success { color: #fff; background: var(--color-success); }
.quest-due-badge.badge-error { color: #fff; background: var(--color-error); }
.quest-due-badge.badge-ghost { color: var(--fg-2); background: var(--color-base-200); }

.quest-repeat-badge {
  color: var(--fg-2);
  background: var(--color-base-200);
}

.quest-space-badge { flex-shrink: 0; border-radius: 5px; }

</style>
