import { mount } from "@vue/test-utils";
import NewQuestForm from "../../components/FocusView/NewQuestForm.vue";
import { useQuestStore } from "../../stores/quest";
import { useSpaceStore } from "../../stores/space";

jest.mock("../../stores/quest", () => ({
  useQuestStore: jest.fn(),
}));

jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  SPACE_COLOR_CLASS: {
    "1": "space-color-personal",
    "2": "space-color-family",
  },
}));

jest.mock("../../composables/useReminderNotifications", () => ({
  useReminderNotifications: () => ({
    ensureReminderNotificationsAllowed: jest.fn().mockResolvedValue(true),
  }),
}));

const reminderPayload = {
  due: "2099-06-15",
  due_time: "14:30",
  repeat_rule: null,
};

describe("NewQuestForm", () => {
  let createQuest: jest.Mock;
  let fetchSpaces: jest.Mock;

  beforeEach(() => {
    createQuest = jest.fn().mockResolvedValue({});
    fetchSpaces = jest.fn().mockResolvedValue(undefined);

    (useQuestStore as unknown as jest.Mock).mockReturnValue({
      createQuest,
    });
    (useSpaceStore as unknown as jest.Mock).mockReturnValue({
      selectedSpaceId: null,
      spaces: [
        { id: "1", name: "Personal" },
        { id: "2", name: "Family" },
      ],
      fetchSpaces,
    });
  });

  it("creates a quest with explicit space and reminder draft fields", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: {
            template: '<button data-testid="stub-reminder-save" @click="$emit(\'save\', payload)">save reminder</button>',
            props: ["quest"],
            emits: ["save", "close"],
            data: () => ({ payload: reminderPayload }),
          },
        },
      },
    });

    await wrapper.find('[data-testid="new-quest-title"]').setValue("Plan the rich composer");
    await wrapper.find('[data-testid="new-quest-space"]').setValue("2");
    await wrapper.find('[data-testid="new-quest-reminder"]').trigger("click");
    await wrapper.find('[data-testid="stub-reminder-save"]').trigger("click");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledWith({
      title: "Plan the rich composer",
      space_id: "2",
      due: "2099-06-15",
      due_time: "14:30",
      repeat_rule: null,
    });
  });

  it("does not create a quest without a title", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find("form").trigger("submit");

    expect(createQuest).not.toHaveBeenCalled();
  });
});
