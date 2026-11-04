import { shallowMount } from "@vue/test-utils";
import { nextTick } from "vue";
import ActiveQuestPanel from "../../components/FocusView/ActiveQuestPanel.vue";
import QuestList from "../../components/QuestsView/QuestList.vue";
import { useQuestStore, type Quest } from "../../stores/quest";
import { EditScope } from "../../utils/quest";

jest.mock("vue-i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

jest.mock("../../stores/quest", () => ({
  ...jest.requireActual("../../stores/quest"),
  useQuestStore: jest.fn(),
}));

jest.mock("../../stores/space", () => ({
  ...jest.requireActual("../../stores/space"),
  useSpaceStore: () => ({ spaces: [{ id: "1", name: "Personal" }] }),
}));

jest.mock("../../composables/useContextMenu", () => ({
  useContextMenu: () => ({ open: jest.fn() }),
}));

jest.mock("../../composables/useQuestMenu", () => ({
  useQuestMenu: jest.fn(() => ({ items: { value: [] }, completeMenu: { value: [] } })),
  hasMissedDays: jest.fn(() => false),
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

const DAILY = JSON.stringify({ preset: "daily" });

/** A repeating checklist quest, resolved to its oldest open day (ADR-0012). */
function baseQuest(overrides: Partial<Quest> = {}): Quest {
  return {
    id: "q1",
    space_id: "1",
    title: "Pack bag",
    description: "- [ ] headpones <!--k=a1-->\n- [x] key fob <!--k=a2-->",
    status: "active",
    energy: "medium",
    priority: "medium",
    due: "2026-01-01",
    due_time: null,
    repeat_rule: DAILY,
    completed_at: null,
    order_rank: 0,
    focus_enter_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
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
    updateQuest: jest.fn().mockResolvedValue(undefined),
    completeThroughToday: jest.fn().mockResolvedValue(undefined),
    setFocusQuest: jest.fn().mockResolvedValue(undefined),
    deleteQuest: jest.fn().mockResolvedValue(undefined),
    fetchChecklistActivity: jest.fn().mockResolvedValue([]),
  };
  (useQuestStore as unknown as jest.Mock).mockReturnValue(store);
  return store;
}

const listStubs = {
  QuestListItem: false,
  QuestEditor: questEditorStub,
  RecurrenceScopeSheet: recurrenceScopeSheetStub,
  ReminderMenu: true,
  ArrowPathIcon: true,
  CheckCircleIcon: true,
};

const panelStubs = {
  QuestEditor: questEditorStub,
  RecurrenceScopeSheet: recurrenceScopeSheetStub,
  ReminderMenu: true,
  CheckIcon: true,
};

describe("repeating checklist edit scope (ADR-0012 D8)", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("asks This day / All days before QuestList applies an item text edit", async () => {
    const store = mockQuestStore();
    const wrapper = shallowMount(QuestList, { props: { quests: [baseQuest()] }, global: { stubs: listStubs } });

    await wrapper.find(".quest-row-surface").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a1", "headphones");
    await nextTick();

    expect(store.editChecklistItemText).not.toHaveBeenCalled();
    expect(wrapper.findComponent({ name: "RecurrenceScopeSheet" }).exists()).toBe(true);

    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", EditScope.All);
    await nextTick();

    expect(store.editChecklistItemText).toHaveBeenCalledWith("q1", "a1", "headphones", EditScope.All);
  });

  it("sends an added item with the chosen scope", async () => {
    const store = mockQuestStore();
    const wrapper = shallowMount(QuestList, { props: { quests: [baseQuest()] }, global: { stubs: listStubs } });

    await wrapper.find(".quest-row-surface").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("add-checklist-item", "lunch");
    await nextTick();
    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", EditScope.Day);
    await nextTick();

    expect(store.addChecklistItem).toHaveBeenCalledWith("q1", "lunch", EditScope.Day);
  });

  it("edits a one-time quest's checklist without asking", async () => {
    const store = mockQuestStore();
    const quest = baseQuest({ repeat_rule: null, due: null });
    const wrapper = shallowMount(ActiveQuestPanel, { props: { quest }, global: { stubs: panelStubs } });

    await wrapper.find(".active-quest-title").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("remove-checklist-item", "a2");
    await nextTick();

    expect(wrapper.findComponent({ name: "RecurrenceScopeSheet" }).exists()).toBe(false);
    expect(store.removeChecklistItem).toHaveBeenCalledWith("q1", "a2", EditScope.All);
  });

  it("applies This day through the focused active panel", async () => {
    const store = mockQuestStore();
    const wrapper = shallowMount(ActiveQuestPanel, { props: { quest: baseQuest() }, global: { stubs: panelStubs } });

    await wrapper.find(".active-quest-title").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a1", "headphones");
    await nextTick();
    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", EditScope.Day);
    await nextTick();

    expect(store.editChecklistItemText).toHaveBeenCalledWith("q1", "a1", "headphones", EditScope.Day);
  });

  it("resolves a pending scope action against the quest that opened the sheet, not a quest swapped in while it was open", async () => {
    // Focus can replace activeQuest (after a sync or a reminder) while the sheet is open; the
    // pending edit must not apply to the new quest.
    const otherQuest = baseQuest({ id: "q2", title: "Different quest", description: "- [ ] milk <!--k=b1-->" });
    const store = mockQuestStore();
    const wrapper = shallowMount(ActiveQuestPanel, { props: { quest: baseQuest() }, global: { stubs: panelStubs } });

    await wrapper.find(".active-quest-title").trigger("click");
    wrapper.findComponent({ name: "QuestEditor" }).vm.$emit("edit-checklist-item-text", "a1", "headphones");
    await nextTick();
    await wrapper.setProps({ quest: otherQuest });

    wrapper.findComponent({ name: "RecurrenceScopeSheet" }).vm.$emit("choose", EditScope.Day);
    await nextTick();

    expect(store.editChecklistItemText).toHaveBeenCalledWith("q1", "a1", "headphones", EditScope.Day);
    expect(store.editChecklistItemText).not.toHaveBeenCalledWith("q2", expect.anything(), expect.anything(), expect.anything());
  });
});
