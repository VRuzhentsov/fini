import { mount } from "@vue/test-utils";
import QuestsView from "../../views/QuestsView.vue";
import { useQuestStore } from "../../stores/quest";

jest.mock("../../components/QuestsView/QuestList.vue", () => ({
  __esModule: true,
  default: {
    name: "QuestList",
    template: '<div data-testid="quest-list-stub" />',
  },
}));

jest.mock("../../components/FocusView/NewQuestForm.vue", () => ({
  __esModule: true,
  default: {
    name: "NewQuestForm",
    template: '<div data-testid="new-quest-form-stub" />',
  },
}));

jest.mock("../../stores/quest", () => ({
  useQuestStore: jest.fn(),
}));

describe("QuestsView quest creation", () => {
  beforeEach(() => {
    (useQuestStore as unknown as jest.Mock).mockReturnValue({
      quests: [],
      error: null,
      fetchQuests: jest.fn().mockResolvedValue(undefined),
    });
  });

  it("uses the shared rich quest composer", () => {
    const wrapper = mount(QuestsView, {
      global: {
        stubs: {
          QuestList: true,
          NewQuestForm: { name: "NewQuestForm", template: '<div data-testid="new-quest-form-stub" />' },
        },
      },
    });

    expect(wrapper.find('[data-testid="new-quest-form-stub"]').exists()).toBe(true);
  });
});
