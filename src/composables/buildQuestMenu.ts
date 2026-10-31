import type { MenuItem } from "./useContextMenu";
import { isRepeating, type Quest, type UpdateQuestInput } from "../stores/quest";
import type { Space } from "../stores/space";
import {
  CheckIcon,
  CheckCircleIcon,
  BoltIcon,
  ArrowsRightLeftIcon,
  PlayIcon,
  XMarkIcon,
  TrashIcon,
} from "@heroicons/vue/24/outline";
import { SPACE_COLOR_CLASS } from "../stores/space";
import { localDateStr } from "../utils/date";

interface QuestMenuDeps {
  spaces: Space[];
  updateQuest: (id: string, patch: UpdateQuestInput) => void | Promise<unknown>;
  setFocusQuest: (id: string) => void | Promise<unknown>;
  deleteQuest: (id: string) => void | Promise<unknown>;
  completeThroughToday?: (id: string) => void | Promise<unknown>;
}

/** Whether "Complete through today" has something to close (ADR-0012 D7): an
 * active repeating quest whose shown (oldest open) day is today or earlier. */
export function canCompleteThroughToday(quest: Quest): boolean {
  return (
    quest.status === "active" &&
    isRepeating(quest) &&
    !!quest.due &&
    quest.due <= localDateStr(new Date())
  );
}

/** The "Complete through today" menu item, or nothing when it has nothing to close. */
export function completeThroughTodayItem(
  quest: Quest,
  completeThroughToday: QuestMenuDeps["completeThroughToday"],
): MenuItem[] {
  if (!completeThroughToday || !canCompleteThroughToday(quest)) return [];
  return [
    {
      label: "Complete through today",
      icon: CheckCircleIcon,
      action: () => completeThroughToday(quest.id),
    },
  ];
}

function spaceColorFromCss(cssClass: string | undefined): string | undefined {
  if (!cssClass) return undefined;
  // SPACE_COLOR_CLASS values are like "space-color-work"; map to var names
  const match = cssClass.match(/space-color-(\w+)/);
  if (!match) return undefined;
  return `var(--space-color-${match[1]})`;
}

export function buildQuestMenu(quest: Quest, deps: QuestMenuDeps): MenuItem[] {
  const currentSpace = deps.spaces.find((s) => s.id === quest.space_id);

  const moveChildren = deps.spaces.map<MenuItem>((s) => ({
    label: s.name,
    selected: s.id === quest.space_id,
    spaceColor: spaceColorFromCss(SPACE_COLOR_CLASS[s.id]),
    action: s.id === quest.space_id ? undefined : () => deps.updateQuest(quest.id, { space_id: s.id }),
  }));

  const moveItem: MenuItem =
    moveChildren.length > 0
      ? {
          label: "Move to space",
          icon: ArrowsRightLeftIcon,
          value: currentSpace?.name,
          children: moveChildren,
        }
      : { label: "Move to space", icon: ArrowsRightLeftIcon, disabled: true };

  const items: MenuItem[] = [];

  // A closed row of a repeating quest is one day of it (ADR-0012): "Make
  // active" opens that day again, and the quest itself is not deleted from it.
  const closedDay = quest.status !== "active" && isRepeating(quest);

  if (quest.status === "active") {
    items.push({ label: "Complete", icon: CheckIcon, action: () => deps.updateQuest(quest.id, { status: "completed" }) });
    items.push(...completeThroughTodayItem(quest, deps.completeThroughToday));
    items.push({ label: "Set Focus", icon: BoltIcon, action: () => deps.setFocusQuest(quest.id) });
    items.push(moveItem);
    items.push({ separator: true });
    items.push({ label: "Abandon", icon: XMarkIcon, action: () => deps.updateQuest(quest.id, { status: "abandoned" }) });
  } else if (closedDay) {
    items.push({
      label: "Make active",
      icon: PlayIcon,
      action: () => deps.updateQuest(quest.id, { status: "active", period_key: quest.due ?? undefined }),
    });
    return items;
  } else {
    items.push({ label: "Make active", icon: PlayIcon, action: () => deps.updateQuest(quest.id, { status: "active" }) });
    items.push(moveItem);
  }

  items.push({ separator: true });
  items.push({ label: "Delete", icon: TrashIcon, danger: true, action: () => deps.deleteQuest(quest.id) });

  return items;
}
