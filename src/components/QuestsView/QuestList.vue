<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useQuestStore, type Quest, type UpdateQuestInput } from "../../stores/quest";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../../stores/space";
import { useContextMenu } from "../../composables/useContextMenu";
import { buildQuestMenu } from "../../composables/buildQuestMenu";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import { ArrowPathIcon, ChevronRightIcon, TrashIcon } from "@heroicons/vue/24/outline";
import ReminderMenu from "./ReminderMenu.vue";
import QuestEditor from "../QuestEditor.vue";

const props = defineProps<{
  quests: Quest[];
  groupChildrenById?: Record<string, Quest[]>;
}>();
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

// ── Group helpers ─────────────────────────────────────────────────────────────

function getGroupChildren(questId: string): Quest[] | null {
  const children = props.groupChildrenById?.[questId];
  return children?.length ? children : null;
}

function rowStatusClass(quest: Quest): string {
  const children = getGroupChildren(quest.id);
  if (!children) return quest.status;
  const hasCompleted = children.some((c) => c.status === "completed");
  const hasAbandoned = children.some((c) => c.status === "abandoned");
  return hasCompleted && hasAbandoned ? "mixed" : children[0].status;
}

function rowStatusLabel(quest: Quest): string {
  const children = getGroupChildren(quest.id);
  if (!children) return statusLabel(quest);
  const completed = children.filter((c) => c.status === "completed").length;
  const abandoned = children.filter((c) => c.status === "abandoned").length;
  if (completed > 0 && abandoned > 0) return `Mixed ${completed} / ${abandoned}`;
  return completed > 0 ? "Completed" : "Abandoned";
}

// ── Context menu ──────────────────────────────────────────────────────────────

function onContextMenu(e: MouseEvent, quest: Quest) {
  const children = getGroupChildren(quest.id);
  if (children) {
    contextMenu.open(e, [
      { label: "Make active latest", icon: ArrowPathIcon, action: () => { void store.updateQuest(children[0].id, { status: "active" }); } },
      { separator: true },
      { label: "Delete series", icon: TrashIcon, danger: true, action: () => { void deleteSeriesConfirm(quest); } },
    ]);
    return;
  }
  const items = buildQuestMenu(quest, {
    spaces: spaceStore.spaces,
    updateQuest: (id, patch) => store.updateQuest(id, patch),
    setFocusQuest: (id) => store.setFocusQuest(id),
    deleteQuest: (id) => store.deleteQuest(id),
  });
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

// ── History / group actions ───────────────────────────────────────────────────

async function restore(id: string) {
  await store.updateQuest(id, { status: "active" });
}

function onCheckClick(quest: Quest) {
  const children = getGroupChildren(quest.id);
  if (children?.length) {
    void store.updateQuest(children[0].id, { status: "active" });
  } else {
    void restore(quest.id);
  }
}

async function deleteSeriesConfirm(quest: Quest) {
  const ok = window.confirm(
    `Delete entire series "${quest.title}"? This removes every past and future occurrence and cannot be undone.`,
  );
  if (!ok) return;
  await store.deleteQuestSeries(quest.series_id!);
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
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
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
        v-if="expandedId !== quest.id || getGroupChildren(quest.id)"
        class="quest-row-surface"
        :class="{ 'is-expanded-group': expandedId === quest.id && getGroupChildren(quest.id) }"
        @click="toggle(quest.id)"
        @contextmenu="onContextMenu($event, quest)"
        :data-testid="getGroupChildren(quest.id) ? 'quest-row-group-header' : undefined"
      >
        <!-- History/group: status glyph restores latest; Active: empty square completes -->
        <button
          v-if="quest.status !== 'active'"
          class="quest-check"
          :class="rowStatusClass(quest)"
          :aria-label="`Make ${quest.title} active`"
          @click.prevent.stop="onCheckClick(quest)"
        />
        <button v-else class="quest-check" :aria-label="`Complete ${quest.title}`" @click.stop="completeQuest(quest.id)" />

        <span
          v-if="quest.status !== 'active'"
          class="quest-status-badge"
          :class="rowStatusClass(quest)"
        >{{ rowStatusLabel(quest) }} · {{ formatTimestamp(quest) }}</span>

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

        <!-- Expand chevron for group rows only -->
        <ChevronRightIcon
          v-if="getGroupChildren(quest.id)"
          class="quest-group-chevron size-3.5"
          :class="{ 'rotate-90': expandedId === quest.id }"
          data-testid="quest-row-group-expander"
        />
      </div>

      <!-- Expanded: group children list (lazy-rendered; collapsing fully unmounts them) -->
      <ul
        v-if="expandedId === quest.id && getGroupChildren(quest.id)"
        class="group-children"
        data-testid="quest-row-group-children"
      >
        <li
          v-for="child in getGroupChildren(quest.id)!"
          :key="child.id"
          class="group-child-row"
        >
          <button
            class="quest-check"
            :class="child.status"
            :aria-label="`Make ${child.title} active`"
            @click.stop="restore(child.id)"
          />
          <span class="quest-status-badge" :class="child.status">{{ statusLabel(child) }} · {{ formatTimestamp(child) }}</span>
          <span class="quest-title" :class="child.status">{{ child.title }}</span>
          <span class="quest-space-badge badge badge-xs" :class="spaceCss(child)">{{ spaceName(child) }}</span>
        </li>
      </ul>

      <!-- Expanded: full editor for standalone quests -->
      <QuestEditor
        v-else-if="expandedId === quest.id"
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
.quest-row-surface.is-expanded-group { background: var(--color-base-200); }

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

.quest-status-badge.mixed {
  color: var(--fg-3);
  background: var(--color-base-200);
}

.quest-due-badge.badge-success { color: #fff; background: var(--color-success); }
.quest-due-badge.badge-error { color: #fff; background: var(--color-error); }
.quest-due-badge.badge-ghost { color: var(--fg-2); background: var(--color-base-200); }

.quest-repeat-badge {
  color: var(--fg-2);
  background: var(--color-base-200);
}

.quest-space-badge { flex-shrink: 0; border-radius: 5px; }

.quest-group-chevron {
  flex-shrink: 0;
  color: var(--fg-4);
  transition: transform var(--dur-normal);
}

.group-children {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.25rem 0 0.25rem 1.625rem;
  margin: 0;
  list-style: none;
}

.group-child-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.3125rem 0.75rem;
  border-radius: 6px;
}

.group-child-row:hover { background: var(--color-base-200); }

</style>
