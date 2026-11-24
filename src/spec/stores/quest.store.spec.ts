import { invoke } from "@tauri-apps/api/core";
import { createPinia, setActivePinia } from "pinia";
import { useQuestStore, type Quest } from "../../stores/quest";
import { EditScope } from "../../utils/quest";

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
    priority: "medium",
    due: null,
    due_time: null,
    repeat_rule: null,
    completed_at: null,
    order_rank: "a0",
    focus_enter_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
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

  it("checklist edits pass the edit scope through to the backend command", async () => {
    const store = useQuestStore();
    (invoke as unknown as jest.Mock).mockResolvedValueOnce(baseQuest());

    await store.editChecklistItemText("q1", "a1", "over-ear headphones", EditScope.Day);

    expect(invoke).toHaveBeenCalledWith("edit_checklist_item", {
      questId: "q1",
      itemId: "a1",
      text: "over-ear headphones",
      scope: EditScope.Day,
    });
  });

  it("completeThroughToday invokes complete_quest_through_today and refreshes", async () => {
    const store = useQuestStore();
    (invoke as unknown as jest.Mock)
      .mockResolvedValueOnce(baseQuest({ repeat_rule: '{"preset":"daily"}', due: "2026-01-02" }))
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce(null);

    await store.completeThroughToday("q1");

    expect(invoke).toHaveBeenNthCalledWith(1, "complete_quest_through_today", { id: "q1" });
    expect(invoke).toHaveBeenNthCalledWith(2, "get_quests");
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

describe("quest store order", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (invoke as unknown as jest.Mock).mockReset();
  });

  function orderQuest(id: string, overrides: Partial<Quest> = {}): Quest {
    return baseQuest({ id, title: id, description: null, is_checklist: false, ...overrides });
  }

  function activeIds(store: ReturnType<typeof useQuestStore>): string[] {
    return store.activeQuests.map((quest) => quest.id);
  }

  it("orders overdue first, then by rank key as bytes, not by the due date", () => {
    const store = useQuestStore();
    store.quests = [
      orderQuest("upper", { order_rank: "a0V" }),
      orderQuest("dated", { order_rank: "a1", due: "2999-01-01" }),
      orderQuest("lower", { order_rank: "Zz" }),
      orderQuest("late", { order_rank: "b00", due: "2000-01-01" }),
      orderQuest("done", { order_rank: "A0", status: "completed" }),
    ];

    // "Zz" < "a0V" < "a1" in byte order; a locale order would put "a0V" first.
    expect(activeIds(store)).toEqual(["late", "lower", "upper", "dated"]);
  });

  it("breaks a tie of rank keys by Priority, then the oldest, then the id", () => {
    const store = useQuestStore();
    store.quests = [
      orderQuest("b-new", { created_at: "2026-02-01T00:00:00Z" }),
      orderQuest("a-new", { created_at: "2026-02-01T00:00:00Z" }),
      orderQuest("old", { created_at: "2026-01-01T00:00:00Z" }),
      orderQuest("urgent", { priority: "high", created_at: "2026-03-01T00:00:00Z" }),
      orderQuest("unreadable", { created_at: "not a time" }),
    ];

    expect(activeIds(store)).toEqual(["urgent", "old", "a-new", "b-new", "unreadable"]);
  });

  it("moveQuest shows the move at once and applies the quests that the backend wrote", async () => {
    const store = useQuestStore();
    store.quests = [
      orderQuest("a", { order_rank: "a0" }),
      orderQuest("b", { order_rank: "a1" }),
      orderQuest("c", { order_rank: "a2" }),
    ];
    let resolveInvoke!: (quests: Quest[]) => void;
    (invoke as unknown as jest.Mock).mockReturnValueOnce(
      new Promise<Quest[]>((resolve) => {
        resolveInvoke = resolve;
      }),
    );

    const pending = store.moveQuest("c", { before: "a" });
    expect(activeIds(store)).toEqual(["c", "a", "b"]);

    resolveInvoke([orderQuest("c", { order_rank: "Zz" })]);
    await pending;

    expect(invoke).toHaveBeenCalledWith("move_quest", { id: "c", placement: { before: "a" } });
    expect(store.quests.find((quest) => quest.id === "c")?.order_rank).toBe("Zz");
    expect(activeIds(store)).toEqual(["c", "a", "b"]);
  });

  it("moveQuest goes back to the old order when the backend rejects the move", async () => {
    const store = useQuestStore();
    store.quests = [orderQuest("a", { order_rank: "a0" }), orderQuest("b", { order_rank: "a1" })];
    (invoke as unknown as jest.Mock).mockRejectedValueOnce(new Error("quest b not found"));

    await expect(store.moveQuest("a", { after: "b" })).rejects.toThrow("quest b not found");

    expect(activeIds(store)).toEqual(["a", "b"]);
  });
});
