import { computed, toValue, type MaybeRefOrGetter } from "vue";
import {
  CheckIcon,
  CheckCircleIcon,
  BoltIcon,
  ArrowsRightLeftIcon,
  PlayIcon,
  XMarkIcon,
  TrashIcon,
} from "@heroicons/vue/24/outline";
import type { MenuItem } from "./useContextMenu";
import { isRepeating, useQuestStore, type Quest, type UpdateQuestInput } from "../stores/quest";
import { SPACE_COLOR_CLASS, useSpaceStore } from "../stores/space";
import { QuestStatus } from "../utils/quest";
import { localDateStr } from "../utils/date";

/** Whether "Complete through today" has something to close (ADR-0012 D7): an
 * active repeating quest whose shown (oldest open) day is today or earlier. */
export function hasMissedDays(quest: Quest): boolean {
  return (
    quest.status === QuestStatus.Active &&
    isRepeating(quest) &&
    !!quest.due &&
    quest.due <= localDateStr(new Date())
  );
}

/** A closed row of a repeating quest is one day of it (ADR-0012): "Make
 * active" opens that day again, and the quest itself is not deleted from it. */
function isClosedDay(quest: Quest): boolean {
  return quest.status !== QuestStatus.Active && isRepeating(quest);
}

function reopenPatch(quest: Quest): UpdateQuestInput {
  if (isClosedDay(quest)) return { status: QuestStatus.Active, period_key: quest.due ?? undefined };
  return { status: QuestStatus.Active };
}

function spaceColorFromCss(cssClass: string | undefined): string | undefined {
  // SPACE_COLOR_CLASS values are like "space-color-work"; map to var names
  const match = cssClass?.match(/space-color-(\w+)/);
  return match ? `var(--space-color-${match[1]})` : undefined;
}

/** The menu of one quest, written out in full; each item says when it shows. */
export function useQuestMenu(source: MaybeRefOrGetter<Quest>) {
  const store = useQuestStore();
  const spaceStore = useSpaceStore();

  const quest = computed(() => toValue(source));
  const active = computed(() => quest.value.status === QuestStatus.Active);
  const closedDay = computed(() => isClosedDay(quest.value));

  const update = (patch: UpdateQuestInput) => store.updateQuest(quest.value.id, patch);

  const complete = computed<MenuItem>(() => ({
    label: "Complete",
    icon: CheckIcon,
    visible: active.value,
    action: () => update({ status: QuestStatus.Completed }),
  }));

  const completeThroughToday = computed<MenuItem>(() => ({
    label: "Complete through today",
    icon: CheckCircleIcon,
    visible: hasMissedDays(quest.value),
    action: () => store.completeThroughToday(quest.value.id),
  }));

  const moveToSpace = computed<MenuItem>(() => ({
    label: "Move to space",
    icon: ArrowsRightLeftIcon,
    visible: !closedDay.value,
    value: spaceStore.spaces.find((s) => s.id === quest.value.space_id)?.name,
    disabled: spaceStore.spaces.length === 0,
    children: spaceStore.spaces.map((s) => ({
      label: s.name,
      selected: s.id === quest.value.space_id,
      spaceColor: spaceColorFromCss(SPACE_COLOR_CLASS[s.id]),
      action: s.id === quest.value.space_id ? undefined : () => update({ space_id: s.id }),
    })),
  }));

  /** The context menu of the quest. */
  const items = computed<MenuItem[]>(() => [
    complete.value,
    completeThroughToday.value,
    { label: "Set Focus", icon: BoltIcon, visible: active.value, action: () => store.setFocusQuest(quest.value.id) },
    { label: "Make active", icon: PlayIcon, visible: !active.value, action: () => update(reopenPatch(quest.value)) },
    moveToSpace.value,
    { separator: true, visible: active.value },
    { label: "Abandon", icon: XMarkIcon, visible: active.value, action: () => update({ status: QuestStatus.Abandoned }) },
    { separator: true, visible: !closedDay.value },
    { label: "Delete", icon: TrashIcon, danger: true, visible: !closedDay.value, action: () => store.deleteQuest(quest.value.id) },
  ]);

  /** The menu of the Complete button on Focus (ADR-0012 D7). */
  const completeMenu = computed<MenuItem[]>(() => [complete.value, completeThroughToday.value]);

  return { items, completeMenu };
}
