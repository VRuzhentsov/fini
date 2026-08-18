<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  useQuestStore,
  type ChecklistActivity,
  type Quest,
  type UpdateQuestInput,
} from "../../stores/quest";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../../stores/space";
import { useContextMenu } from "../../composables/useContextMenu";
import { buildQuestMenu } from "../../composables/buildQuestMenu";
import { useReminderNotifications } from "../../composables/useReminderNotifications";
import { ArrowPathIcon, CheckCircleIcon } from "@heroicons/vue/24/outline";
import { checklistCounts, newChecklistItemId, parseChecklist, serializeChecklist } from "../../utils/checklist";
import ReminderMenu from "./ReminderMenu.vue";
import RecurrenceScopeSheet from "./RecurrenceScopeSheet.vue";
import QuestEditor from "../QuestEditor.vue";
import QuestMetadataButton from "../QuestMetadataButton.vue";

const props = defineProps<{
  quest: Quest;
  expanded: boolean;
}>();
const emit = defineEmits<{
  toggle: [];
}>();

const store = useQuestStore();
const { t } = useI18n();
const spaceStore = useSpaceStore();
const contextMenu = useContextMenu();
const { ensureReminderNotificationsAllowed } = useReminderNotifications();

function spaceName(): string {
  return spaceStore.spaces.find((s) => s.id === props.quest.space_id)?.name ?? "";
}

function spaceCss(): string {
  return SPACE_COLOR_CLASS[props.quest.space_id] ?? "";
}

function statusLabel(): string {
  return props.quest.status === "completed" ? "Completed" : "Abandoned";
}

// ── Context menu ──────────────────────────────────────────────────────────────

function onContextMenu(e: MouseEvent) {
  const items = buildQuestMenu(props.quest, {
    spaces: spaceStore.spaces,
    updateQuest: (id, patch) => store.updateQuest(id, patch),
    setFocusQuest: (id) => store.setFocusQuest(id),
    deleteQuest: (id) => store.deleteQuest(id),
  });
  contextMenu.open(e, items);
}

// ── Checklist (issue #128) ──────────────────────────────────────────────────

const checklistActivity = ref<ChecklistActivity[] | undefined>(undefined);

// History rows show the completion-time audit trail; active rows have nothing to audit yet.
watch(
  () => props.expanded,
  (expanded) => {
    if (expanded && props.quest.is_checklist && props.quest.status !== "active" && !checklistActivity.value) {
      store.fetchChecklistActivity(props.quest.id).then((activity) => {
        checklistActivity.value = activity;
      });
    }
  },
);

function checklistBadgeText(): string {
  const [done, total] = checklistCounts(props.quest.description);
  return `${done}/${total}`;
}

function onToggleChecklistItem(itemId: string, checked: boolean) {
  // Checking off "today's" packing is always this-occurrence-only — future occurrences always
  // start fresh and unchecked regardless (see recurring template copy), so there's nothing to
  // ask the user about scope here.
  store.toggleChecklistItem(props.quest.id, itemId, checked);
}

function onEditChecklistItemText(itemId: string, text: string) {
  if (props.quest.series_id) {
    pendingScopeAction.value = { quest: props.quest, kind: "edit", payload: { itemId, text } };
    return;
  }
  store.editChecklistItemText(props.quest.id, itemId, text);
}

// A structural or text edit on a recurring quest needs the user's scope choice
// (#128: "This occurrence" vs "This and future occurrences") before it's applied.
type PendingScopeAction =
  | { quest: Quest; kind: "add"; payload: string }
  | { quest: Quest; kind: "remove"; payload: string }
  | { quest: Quest; kind: "edit"; payload: { itemId: string; text: string } };

const pendingScopeAction = ref<PendingScopeAction | null>(null);

function onAddChecklistItem(text: string) {
  if (props.quest.series_id) {
    pendingScopeAction.value = { quest: props.quest, kind: "add", payload: text };
    return;
  }
  store.addChecklistItem(props.quest.id, text);
}

function onRemoveChecklistItem(itemId: string) {
  if (props.quest.series_id) {
    pendingScopeAction.value = { quest: props.quest, kind: "remove", payload: itemId };
    return;
  }
  store.removeChecklistItem(props.quest.id, itemId);
}

async function onScopeChosen(scope: "this" | "future") {
  const action = pendingScopeAction.value;
  pendingScopeAction.value = null;
  if (!action) return;
  const { quest, kind, payload } = action;

  if (scope === "this") {
    if (kind === "add") await store.addChecklistItem(quest.id, payload);
    else if (kind === "remove") await store.removeChecklistItem(quest.id, payload);
    else await store.editChecklistItemText(quest.id, payload.itemId, payload.text);
    return;
  }

  // "This and future occurrences": diff against the series' own stored template, not this
  // occurrence's current description (which may already carry "this occurrence only" changes
  // that were never promoted — basing the edit on it would silently promote them), then push the
  // result as the new template. The backend reconciles this occurrence against it (preserving
  // checks on unchanged items, per #128).
  const template = await store.fetchSeriesChecklistTemplate(quest.series_id!);
  const items = parseChecklist(template);

  if (kind === "edit" && !items.some((it) => it.id === payload.itemId)) {
    // The item being renamed was added via "This occurrence" and was never promoted to the
    // template — it has no counterpart there to update. Pushing the template unchanged would
    // leave the rename un-applied, and reconcile_future_scope would then drop the item entirely
    // (since it's absent from the new template), destroying it instead of renaming it. There's no
    // coherent "future" version of an occurrence-only item, so fall back to applying the rename
    // to just this occurrence.
    await store.editChecklistItemText(quest.id, payload.itemId, payload.text);
    return;
  }

  const nextItems =
    kind === "add"
      ? [...items, { id: newChecklistItemId(), text: payload, checked: false }]
      : kind === "remove"
        ? items.filter((it) => it.id !== payload)
        : items.map((it) => (it.id === payload.itemId ? { ...it, text: payload.text } : it));
  await store.updateSeriesChecklist(quest.series_id!, quest.id, serializeChecklist(nextItems), "future");
}

// ── Active actions ────────────────────────────────────────────────────────────

async function completeQuest() {
  await store.updateQuest(props.quest.id, { status: "completed" });
}

async function setFocus() {
  await store.setFocusQuest(props.quest.id);
}

async function updateQuest(patch: UpdateQuestInput) {
  await store.updateQuest(props.quest.id, patch);
}

// ── History actions ───────────────────────────────────────────────────────────

async function restore() {
  await store.updateQuest(props.quest.id, { status: "active" });
}

// ── Priority ──────────────────────────────────────────────────────────────────

const PRIORITIES = ["low", "medium", "high"] as const;
const PRIORITY_LABELS: Record<Quest["priority"], string> = { low: "Low", medium: "Medium", high: "High" };
const PRIORITY_COLORS: Record<Quest["priority"], string> = { low: "oklch(var(--color-success))", medium: "oklch(var(--color-warning))", high: "oklch(var(--color-error))" };

async function cyclePriority() {
  const idx = PRIORITIES.indexOf(props.quest.priority);
  const next = PRIORITIES[(idx + 1) % PRIORITIES.length];
  await store.updateQuest(props.quest.id, { priority: next });
}

// ── Reminder menu ─────────────────────────────────────────────────────────────

const reminderOpen = ref(false);

async function onReminderSave(payload: { due: string | null; due_time: string | null; repeat_rule: string | null }) {
  if (!(await ensureReminderNotificationsAllowed(payload))) {
    return;
  }
  await store.updateQuest(props.quest.id, payload);
  reminderOpen.value = false;
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

function smartDueLabel(): string {
  if (!props.quest.due) return "";
  const now = new Date();
  const todayStr = localDateStr(now);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const tomorrowStr = localDateStr(tomorrow);
  const time = props.quest.due_time ? `, ${formatTime(props.quest.due_time)}` : "";
  if (props.quest.due === todayStr) return t("quest.today") + time;
  if (props.quest.due === tomorrowStr) return t("quest.tomorrow") + time;
  return formatDue(props.quest.due) + time;
}

function dueBadgeClass(): string {
  if (!props.quest.due) return "";
  const todayStr = localDateStr(new Date());
  if (props.quest.due < todayStr) return "badge-error";
  if (props.quest.due === todayStr) return "badge-success";
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

function pillText(): string {
  const parts: string[] = [];
  if (props.quest.due) {
    const date = formatDue(props.quest.due);
    const time = props.quest.due_time ? ` at ${formatTime(props.quest.due_time)}` : "";
    parts.push(date + time);
  }
  if (props.quest.repeat_rule) {
    const r = formatRepeat(props.quest.repeat_rule);
    if (r) parts.push(r);
  }
  return parts.join(", ");
}

function formatTimestamp(): string {
  const raw = props.quest.completed_at ?? props.quest.updated_at;
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
  <!-- Collapsed row -->
  <div
    v-if="!expanded"
    class="quest-row-surface"
    @click="emit('toggle')"
    @contextmenu="onContextMenu"
  >
    <!-- History: check glyph restores; Active: empty square completes -->
    <button
      v-if="quest.status !== 'active'"
      class="quest-check"
      :class="quest.status"
      :aria-label="`Make ${quest.title} active`"
      @click.prevent.stop="restore"
    />
    <button v-else class="quest-check" :aria-label="`Complete ${quest.title}`" @click.stop="completeQuest" />

    <span
      v-if="quest.status !== 'active'"
      class="quest-status-badge"
      :class="quest.status"
    >{{ statusLabel() }} · {{ formatTimestamp() }}</span>

    <span v-if="quest.due && quest.status === 'active'" class="quest-due-badge" :class="dueBadgeClass()">
      {{ smartDueLabel() }}
      <ArrowPathIcon v-if="quest.repeat_rule" class="size-3.5" />
    </span>
    <span v-else-if="quest.repeat_rule && quest.status === 'active'" class="quest-repeat-badge">
      <ArrowPathIcon class="size-3.5" />
      {{ formatRepeat(quest.repeat_rule) }}
    </span>
    <span class="quest-title" :class="quest.status !== 'active' ? quest.status : ''">{{ quest.title }}</span>
    <span v-if="quest.is_checklist" class="quest-checklist-badge">
      <CheckCircleIcon class="size-3" />
      {{ checklistBadgeText() }}
    </span>
    <QuestMetadataButton v-if="quest.status === 'active' && quest.energy !== 'medium'" kind="energy" :model-value="quest.energy" readonly />
    <QuestMetadataButton v-if="quest.status === 'active' && quest.priority !== 'medium'" kind="priority" :model-value="quest.priority" readonly />
    <span class="quest-space-badge badge badge-xs" :class="spaceCss()">{{ spaceName() }}</span>
  </div>

  <!-- Expanded: full editor for standalone quests -->
  <QuestEditor
    v-else
    :quest="quest"
    :space-name="spaceName()"
    :is-focus="store.activeQuest?.id === quest.id"
    :priority-color="PRIORITY_COLORS[quest.priority]"
    :priority-label="PRIORITY_LABELS[quest.priority]"
    :reminder-text="pillText()"
    :timestamp-text="quest.status !== 'active' ? formatTimestamp() : ''"
    :is-recurring="!!quest.series_id"
    :checklist-activity="checklistActivity"
    @contextmenu="onContextMenu"
    @update="updateQuest"
    @complete="completeQuest"
    @restore="restore"
    @set-focus="setFocus"
    @collapse="emit('toggle')"
    @open-reminder="reminderOpen = true"
    @cycle-priority="cyclePriority"
    @more="onContextMenu"
    @toggle-checklist-item="onToggleChecklistItem"
    @add-checklist-item="onAddChecklistItem"
    @edit-checklist-item-text="onEditChecklistItemText"
    @remove-checklist-item="onRemoveChecklistItem"
  />

  <!-- Reminder menu (active only) -->
  <ReminderMenu
    v-if="reminderOpen"
    :quest="quest"
    @close="reminderOpen = false"
    @save="onReminderSave"
  />

  <!-- Recurrence checklist edit-scope prompt (issue #128) -->
  <RecurrenceScopeSheet
    v-if="pendingScopeAction"
    :quest-title="pendingScopeAction.quest.title"
    @close="pendingScopeAction = null"
    @choose="onScopeChosen"
  />
</template>

<style scoped>
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

.quest-checklist-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  flex-shrink: 0;
  padding: 0.125rem 0.375rem;
  font-family: var(--font-mono);
  font-size: 0.625rem;
  color: var(--fg-3);
  background: var(--color-base-200);
  border-radius: 6px;
}
</style>
