import { createPinia, setActivePinia } from "pinia";
import { useQuestMenu } from "../../composables/useQuestMenu";
import type { MenuItem } from "../../composables/useContextMenu";
import { useQuestStore, type Quest } from "../../stores/quest";
import { QuestStatus } from "../../utils/quest";
import { localDateStr } from "../../utils/date";

jest.mock("@tauri-apps/api/core", () => ({
  invoke: jest.fn(),
}));

const TODAY = localDateStr(new Date());
const DAILY = "daily";

function quest(fields: Partial<Quest>): Quest {
  return {
    id: "q1",
    title: "Water the plants",
    status: QuestStatus.Active,
    space_id: "1",
    repeat_rule: null,
    due: null,
    ...fields,
  } as Quest;
}

function shownLabels(items: MenuItem[]): string[] {
  return items.filter((i) => i.visible !== false && i.label).map((i) => i.label as string);
}

function item(items: MenuItem[], label: string): MenuItem {
  const found = items.find((i) => i.label === label);
  if (!found) throw new Error(`no menu item ${label}`);
  return found;
}

describe("useQuestMenu", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("shows Complete through today for a repeating quest with a missed day", () => {
    const { items, completeMenu } = useQuestMenu(quest({ repeat_rule: DAILY, due: "2020-01-01" }));
    expect(shownLabels(items.value)).toContain("Complete through today");
    expect(shownLabels(completeMenu.value)).toEqual(["Complete", "Complete through today"]);
  });

  it("shows it when the open day is today, since today's day is closed too", () => {
    const { items } = useQuestMenu(quest({ repeat_rule: DAILY, due: TODAY }));
    expect(shownLabels(items.value)).toContain("Complete through today");
  });

  it("hides it for a one-time quest", () => {
    const { items } = useQuestMenu(quest({ due: "2020-01-01" }));
    expect(shownLabels(items.value)).not.toContain("Complete through today");
  });

  it("hides it when only future days are open", () => {
    const { items } = useQuestMenu(quest({ repeat_rule: DAILY, due: "2999-01-01" }));
    expect(shownLabels(items.value)).not.toContain("Complete through today");
  });

  it("calls the store's completeThroughToday for the quest", () => {
    const store = useQuestStore();
    const spy = jest.spyOn(store, "completeThroughToday").mockResolvedValue(undefined as never);
    const { items } = useQuestMenu(quest({ repeat_rule: DAILY, due: "2020-01-01" }));
    item(items.value, "Complete through today").action?.();
    expect(spy).toHaveBeenCalledWith("q1");
  });

  it("a closed day of a repeating quest shows only Make active, which opens that day", () => {
    const store = useQuestStore();
    const spy = jest.spyOn(store, "updateQuest").mockResolvedValue(undefined as never);
    const { items } = useQuestMenu(
      quest({ repeat_rule: DAILY, due: "2020-01-01", status: QuestStatus.Completed }),
    );
    expect(shownLabels(items.value)).toEqual(["Make active"]);
    item(items.value, "Make active").action?.();
    expect(spy).toHaveBeenCalledWith("q1", { status: QuestStatus.Active, period_key: "2020-01-01" });
  });
});
