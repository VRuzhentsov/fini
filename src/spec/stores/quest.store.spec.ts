import { invoke } from "@tauri-apps/api/core";
import { createPinia, setActivePinia } from "pinia";
import { useQuestStore, type Quest } from "../../stores/quest";

jest.mock("@tauri-apps/api/core", () => ({
  invoke: jest.fn(),
}));

function baseQuest(overrides: Partial<Quest> = {}): Quest {
  return {
    id: "q1",
    space_id: "1",
    title: "Go to office",
    description: "- [ ] headphones <!--k=a1-->\n- [ ] key fob <!--k=a2-->",
    status: "active",
    energy: "medium",
    priority: 1,
    pinned: false,
    due: null,
    due_time: null,
    repeat_rule: null,
    completed_at: null,
    order_rank: 0,
    focus_enter_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    series_id: null,
    period_key: null,
    is_checklist: true,
    ...overrides,
  };
}

describe("quest store checklist actions", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (invoke as unknown as jest.Mock).mockReset();
  });

  it("optimistically toggles the item before the server responds, then reconciles", async () => {
    const store = useQuestStore();
    store.quests = [baseQuest()];

    let resolveInvoke!: (quest: Quest) => void;
    (invoke as unknown as jest.Mock).mockReturnValueOnce(
      new Promise<Quest>((resolve) => {
        resolveInvoke = resolve;
      }),
    );

    const pending = store.toggleChecklistItem("q1", "a1", true);

    // Optimistic update happens synchronously, before the mocked invoke resolves.
    expect(store.quests[0].description).toContain("[x] headphones");

    resolveInvoke(
      baseQuest({
        description: "- [x] headphones <!--k=a1-->\n- [ ] key fob <!--k=a2-->",
        updated_at: "2026-01-01T00:05:00Z",
      }),
    );
    await pending;

    expect(store.quests[0].updated_at).toBe("2026-01-01T00:05:00Z");
    expect(invoke).toHaveBeenCalledWith("toggle_checklist_item", {
      questId: "q1",
      itemId: "a1",
      checked: true,
    });
  });

  it("restores the previous checklist state when an optimistic toggle is rejected", async () => {
    const store = useQuestStore();
    store.quests = [baseQuest()];
    const staleItemError = new Error("checklist item not found");

    let rejectInvoke!: (error: Error) => void;
    (invoke as unknown as jest.Mock).mockReturnValueOnce(
      new Promise<Quest>((_, reject) => {
        rejectInvoke = reject;
      }),
    );

    const pending = store.toggleChecklistItem("q1", "a1", true);
    expect(store.quests[0].description).toContain("[x] headphones");

    rejectInvoke(staleItemError);
    await expect(pending).rejects.toThrow("checklist item not found");

    expect(store.quests[0].description).toBe(
      "- [ ] headphones <!--k=a1-->\n- [ ] key fob <!--k=a2-->",
    );
    expect(store.quests[0].updated_at).toBe("2026-01-01T00:00:00Z");
  });

  it("addChecklistItem invokes add_checklist_item and applies the returned quest", async () => {
    const store = useQuestStore();
    store.quests = [baseQuest()];
    const updated = baseQuest({
      description:
        "- [ ] headphones <!--k=a1-->\n- [ ] key fob <!--k=a2-->\n- [ ] lunch <!--k=a3-->",
    });
    (invoke as unknown as jest.Mock).mockResolvedValueOnce(updated);

    const result = await store.addChecklistItem("q1", "lunch");

    expect(invoke).toHaveBeenCalledWith("add_checklist_item", { questId: "q1", text: "lunch" });
    expect(result).toEqual(updated);
    expect(store.quests[0]).toEqual(updated);
  });

  it("removeChecklistItem invokes remove_checklist_item with questId/itemId", async () => {
    const store = useQuestStore();
    (invoke as unknown as jest.Mock).mockResolvedValueOnce(baseQuest({ description: "" }));

    await store.removeChecklistItem("q1", "a1");

    expect(invoke).toHaveBeenCalledWith("remove_checklist_item", { questId: "q1", itemId: "a1" });
  });

  it("updateSeriesChecklist passes scope through to the backend command", async () => {
    const store = useQuestStore();
    (invoke as unknown as jest.Mock).mockResolvedValueOnce(baseQuest());

    await store.updateSeriesChecklist("series-1", "q1", "- [ ] headphones <!--k=a1-->", "future");

    expect(invoke).toHaveBeenCalledWith("update_series_checklist", {
      seriesId: "series-1",
      currentOccurrenceId: "q1",
      checklist: "- [ ] headphones <!--k=a1-->",
      scope: "future",
    });
  });

  it("fetchChecklistActivity returns the activity list from the backend", async () => {
    const store = useQuestStore();
    const activity = [
      {
        id: "act-1",
        quest_id: "q1",
        kind: "added",
        detail: 'Added "lunch"',
        created_at: "2026-01-01T00:00:00Z",
        origin_device_id: "dev-a",
      },
    ];
    (invoke as unknown as jest.Mock).mockResolvedValueOnce(activity);

    const result = await store.fetchChecklistActivity("q1");

    expect(invoke).toHaveBeenCalledWith("get_checklist_activity", { questId: "q1" });
    expect(result).toEqual(activity);
  });
});
