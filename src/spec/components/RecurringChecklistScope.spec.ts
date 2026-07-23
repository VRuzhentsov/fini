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

function mockQuestStore() {
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
  };
  (useQuestStore as unknown as jest.Mock).mockReturnValue(store);
  return store;
}

describe("recurring checklist item text scope", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("prompts for scope before QuestList applies recurring item text edits", async () => {
    const store = mockQuestStore();
    const quest = baseQuest();
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

    expect(store.updateSeriesChecklist).toHaveBeenCalledWith(
      "series-1",
      "q1",
      "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->",
      "future",
    );
    expect(store.editChecklistItemText).not.toHaveBeenCalled();
  });

  it("applies this-occurrence recurring item text edits through the focused active panel", async () => {
    const store = mockQuestStore();
    const wrapper = shallowMount(ActiveQuestPanel, {
      props: { quest: baseQuest() },
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
});
