import { shallowMount } from "@vue/test-utils";
import { nextTick } from "vue";
import ActiveQuestPanel from "../../components/FocusView/ActiveQuestPanel.vue";
import QuestList from "../../components/QuestsView/QuestList.vue";
import { useQuestStore, type Quest } from "../../stores/quest";

jest.mock("vue-i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

jest.mock("../../stores/quest", () => ({
  useQuestStore: jest.fn(),
}));

jest.mock("../../stores/space", () => ({
  SPACE_COLOR_CLASS: { "1": "space-personal" },
  useSpaceStore: () => ({ spaces: [{ id: "1", name: "Personal" }] }),
}));

jest.mock("../../composables/useContextMenu", () => ({
  useContextMenu: () => ({ open: jest.fn() }),
}));

jest.mock("../../composables/buildQuestMenu", () => ({
  buildQuestMenu: jest.fn(() => []),
}));

jest.mock("../../composables/useReminderNotifications", () => ({
  useReminderNotifications: () => ({ ensureReminderNotificationsAllowed: jest.fn().mockResolvedValue(true) }),
}));

const questEditorStub = {
  name: "QuestEditor",
  template: '<div data-testid="quest-editor-stub" />',
};

const recurrenceScopeSheetStub = {
  name: "RecurrenceScopeSheet",
  template: '<div data-testid="scope-sheet-stub" />',
};

function baseQuest(overrides: Partial<Quest> = {}): Quest {
  return {
    id: "q1",
    space_id: "1",
    title: "Pack bag",
    description: "- [ ] headpones <!--k=a1-->\n- [x] key fob <!--k=a2-->",
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
    series_id: "series-1",
    period_key: "2026-01-01",
    is_checklist: true,
    ...overrides,
  };
}

function mockQuestStore(seriesTemplate: string | null = null) {
  const store = {
    activeQuest: null,
    toggleChecklistItem: jest.fn().mockResolvedValue(undefined),
    editChecklistItemText: jest.fn().mockResolvedValue(undefined),
    addChecklistItem: jest.fn().mockResolvedValue(undefined),
    removeChecklistItem: jest.fn().mockResolvedValue(undefined),
    updateSeriesChecklist: jest.fn().mockResolvedValue(undefined),
    updateQuest: jest.fn().mockResolvedValue(undefined),
    setFocusQuest: jest.fn().mockResolvedValue(undefined),
    deleteQuest: jest.fn().mockResolvedValue(undefined),
    fetchChecklistActivity: jest.fn().mockResolvedValue([]),
    fetchSeriesChecklistTemplate: jest.fn().mockResolvedValue(seriesTemplate),
  };
  (useQuestStore as unknown as jest.Mock).mockReturnValue(store);
  return store;
}

describe("recurring checklist item text scope", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("prompts for scope before QuestList applies recurring item text edits", async () => {
    const quest = baseQuest();
    const store = mockQuestStore(quest.description);
    const wrapper = shallowMount(QuestList, {
      props: { quests: [quest] },
      global: {
        stubs: {
          QuestEditor: questEditorStub,
          RecurrenceScopeSheet: recurrenceScopeSheetStub,
          ReminderMenu: true,
          ArrowPathIcon: true,
          CheckCircleIcon: true,
        },
      },
    });

    await wrapper.find(".quest-row-surface").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a1", "headphones");
    await nextTick();

    expect(store.editChecklistItemText).not.toHaveBeenCalled();
    expect(store.updateSeriesChecklist).not.toHaveBeenCalled();
    expect(wrapper.findComponent({ name: "RecurrenceScopeSheet" }).exists()).toBe(true);

    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", "future");
    await nextTick();
    await nextTick();

    expect(store.fetchSeriesChecklistTemplate).toHaveBeenCalledWith("series-1");
    expect(store.updateSeriesChecklist).toHaveBeenCalledWith(
      "series-1",
      "q1",
      "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->",
      "future",
    );
    expect(store.editChecklistItemText).not.toHaveBeenCalled();
  });

  it("bases a future-scope edit on the series template, not occurrence-only changes (#128)", async () => {
    // The occurrence already has a "today only" item that was never promoted to the template —
    // a future-scope edit must not silently promote it.
    const quest = baseQuest({
      description:
        "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->\n- [ ] today only <!--k=a3-->",
    });
    const seriesTemplate = "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->";
    const store = mockQuestStore(seriesTemplate);
    const wrapper = shallowMount(QuestList, {
      props: { quests: [quest] },
      global: {
        stubs: {
          QuestEditor: questEditorStub,
          RecurrenceScopeSheet: recurrenceScopeSheetStub,
          ReminderMenu: true,
          ArrowPathIcon: true,
          CheckCircleIcon: true,
        },
      },
    });

    await wrapper.find(".quest-row-surface").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("add-checklist-item", "lunch");
    await nextTick();
    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", "future");
    await nextTick();
    await nextTick();

    const [, , sentChecklist] = store.updateSeriesChecklist.mock.calls[0];
    expect(sentChecklist).toContain("lunch");
    expect(sentChecklist).not.toContain("today only");
  });

  it("falls back to a this-occurrence edit when a future-scoped rename targets an occurrence-only item (#128)", async () => {
    // "today only" exists on the occurrence but was never promoted to the series template — there
    // is no "future" version of it to rename.
    const quest = baseQuest({
      description:
        "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->\n- [ ] today only <!--k=a3-->",
    });
    const seriesTemplate = "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->";
    const store = mockQuestStore(seriesTemplate);
    const wrapper = shallowMount(QuestList, {
      props: { quests: [quest] },
      global: {
        stubs: {
          QuestEditor: questEditorStub,
          RecurrenceScopeSheet: recurrenceScopeSheetStub,
          ReminderMenu: true,
          ArrowPathIcon: true,
          CheckCircleIcon: true,
        },
      },
    });

    await wrapper.find(".quest-row-surface").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a3", "today only, renamed");
    await nextTick();
    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", "future");
    await nextTick();
    await nextTick();

    expect(store.editChecklistItemText).toHaveBeenCalledWith("q1", "a3", "today only, renamed");
    expect(store.updateSeriesChecklist).not.toHaveBeenCalled();
  });

  it("applies this-occurrence recurring item text edits through the focused active panel", async () => {
    const quest = baseQuest();
    const store = mockQuestStore(quest.description);
    const wrapper = shallowMount(ActiveQuestPanel, {
      props: { quest },
      global: {
        stubs: {
          QuestEditor: questEditorStub,
          RecurrenceScopeSheet: recurrenceScopeSheetStub,
          ReminderMenu: true,
          CheckIcon: true,
        },
      },
    });

    await wrapper.find(".active-quest-title").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a1", "headphones");
    await nextTick();

    expect(store.editChecklistItemText).not.toHaveBeenCalled();
    expect(wrapper.findComponent({ name: "RecurrenceScopeSheet" }).exists()).toBe(true);

    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", "this");
    await nextTick();

    expect(store.editChecklistItemText).toHaveBeenCalledWith("q1", "a1", "headphones");
    expect(store.updateSeriesChecklist).not.toHaveBeenCalled();
  });

  it("resolves a pending scope action against the quest that opened the sheet, not a quest swapped in while it was open", async () => {
    // Simulates Focus reassigning activeQuest (e.g. after a sync or reminder refresh) while the
    // scope sheet is still open — the pending edit must not silently apply to the new quest.
    const quest = baseQuest();
    const otherQuest = baseQuest({
      id: "q2",
      series_id: "series-2",
      title: "Different quest",
      description: "- [ ] milk <!--k=b1-->",
    });
    const store = mockQuestStore(quest.description);
    const wrapper = shallowMount(ActiveQuestPanel, {
      props: { quest },
      global: {
        stubs: {
          QuestEditor: questEditorStub,
          RecurrenceScopeSheet: recurrenceScopeSheetStub,
          ReminderMenu: true,
          CheckIcon: true,
        },
      },
    });

    await wrapper.find(".active-quest-title").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a1", "headphones");
    await nextTick();

    // The active quest changes out from under the still-open sheet.
    await wrapper.setProps({ quest: otherQuest });

    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", "this");
    await nextTick();

    expect(store.editChecklistItemText).toHaveBeenCalledWith("q1", "a1", "headphones");
    expect(store.editChecklistItemText).not.toHaveBeenCalledWith(
      "q2",
      expect.anything(),
      expect.anything(),
    );
  });
});
