import type { MenuItem } from "./useContextMenu";
import type { Quest, UpdateQuestInput } from "../stores/quest";
import type { Space } from "../stores/space";

interface QuestMenuDeps {
  spaces: Space[];
  updateQuest: (id: string, patch: UpdateQuestInput) => void | Promise<unknown>;
  setFocusQuest: (id: string) => void | Promise<unknown>;
  deleteQuest: (id: string) => void | Promise<unknown>;
}

export function buildQuestMenu(quest: Quest, deps: QuestMenuDeps): MenuItem[] {
  const moveChildren = deps.spaces
    .filter((s) => s.id !== quest.space_id)
    .map<MenuItem>((s) => ({
      label: s.name,
      action: () => deps.updateQuest(quest.id, { space_id: s.id }),
    }));

  const moveItem: MenuItem =
    moveChildren.length > 0
      ? { label: "Move to space", children: moveChildren }
      : { label: "Move to space", disabled: true };

  const items: MenuItem[] = [];

  if (quest.status === "active") {
    items.push({ label: "Complete", action: () => deps.updateQuest(quest.id, { status: "completed" }) });
    items.push({ label: "Set Focus", action: () => deps.setFocusQuest(quest.id) });
    items.push(moveItem);
    items.push({ separator: true });
    items.push({ label: "Abandon", action: () => deps.updateQuest(quest.id, { status: "abandoned" }) });
  } else {
    items.push({ label: "Make active", action: () => deps.updateQuest(quest.id, { status: "active" }) });
    items.push(moveItem);
  }

  items.push({ separator: true });
  items.push({ label: "Delete", action: () => deps.deleteQuest(quest.id) });

  return items;
}
