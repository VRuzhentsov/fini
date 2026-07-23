import { mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";
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

async function chooseSpace(wrapper: ReturnType<typeof mount>, spaceId: string) {
  await wrapper.find('[data-testid="new-quest-space"]').trigger("click");
  await wrapper.find(`[data-space-id="${spaceId}"]`).trigger("click");
}

describe("NewQuestForm", () => {
  let createQuest: jest.Mock;
  let fetchSpaces: jest.Mock;
  let spaceStoreState: {
    selectedSpaceId: string | null;
    spaces: Array<{ id: string; name: string }>;
    fetchSpaces: jest.Mock;
  };

  beforeEach(() => {
    createQuest = jest.fn().mockResolvedValue({
      id: "quest-1",
      space_id: "1",
      title: "Created quest",
      description: null,
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
      created_at: "",
      updated_at: "",
      series_id: null,
      period_key: null,
    });
    fetchSpaces = jest.fn().mockResolvedValue(undefined);
    spaceStoreState = reactive({
      selectedSpaceId: null,
      spaces: [
        { id: "1", name: "Personal" },
        { id: "2", name: "Family" },
      ],
      fetchSpaces,
    });

    (useQuestStore as unknown as jest.Mock).mockReturnValue({
      createQuest,
    });
    (useSpaceStore as unknown as jest.Mock).mockReturnValue(spaceStoreState);
  });

  it("renders as the persistent bottom composer surface", () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    expect(wrapper.find(".chat-composer-bar").exists()).toBe(true);
  });

  it("uses a single-line title input for quest creation", () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    const titleInput = wrapper.find('[data-testid="chat-input"]');

    expect(titleInput.element.tagName).toBe("INPUT");
    expect(titleInput.attributes("type")).toBe("text");
    expect(wrapper.find('[data-testid="chat-input"] textarea').exists()).toBe(false);
  });

  it("opens the bottom composer space menu upward", () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    expect(wrapper.findComponent({ name: "SpacePicker" }).props("menuPlacement")).toBe("top");
  });

  it("creates a quest when Enter is pressed in the title input", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Submit from keyboard");
    await wrapper.find('[data-testid="chat-input"]').trigger("keydown", { key: "Enter" });

    expect(createQuest).toHaveBeenCalledWith({
      title: "Submit from keyboard",
      description: null,
      is_checklist: false,
      space_id: "1",
      due: null,
      due_time: null,
      repeat_rule: null,
    });
  });

  it("starts collapsed and expands metadata without losing the title draft", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Capture fast quest");

    expect(wrapper.find('[data-testid="new-quest-description"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="new-quest-focus-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="new-quest-keep-adding"]').exists()).toBe(false);

    await wrapper.find('[data-testid="new-quest-expand"]').trigger("click");

    expect(wrapper.find('[data-testid="new-quest-description"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="new-quest-focus-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="new-quest-keep-adding"]').exists()).toBe(false);
    expect((wrapper.find('[data-testid="chat-input"]').element as HTMLInputElement).value).toBe("Capture fast quest");
  });

  it("creates a quest with explicit space, description, and reminder draft fields", async () => {
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

    await wrapper.find('[data-testid="chat-input"]').setValue("Plan the rich composer");
    await wrapper.find('[data-testid="new-quest-expand"]').trigger("click");
    await wrapper.find('[data-testid="new-quest-description"]').setValue("Capture the extra notes here.");
    await chooseSpace(wrapper, "2");
    await wrapper.find('[data-testid="new-quest-reminder"]').trigger("click");
    await wrapper.find('[data-testid="stub-reminder-save"]').trigger("click");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledWith({
      title: "Plan the rich composer",
      description: "Capture the extra notes here.",
      is_checklist: false,
      space_id: "2",
      due: "2099-06-15",
      due_time: "14:30",
      repeat_rule: null,
    });
  });

  async function addChecklistDraftItem(wrapper: ReturnType<typeof mount>, text: string) {
    const input = wrapper.find('[data-testid="new-quest-checklist-item-input"]');
    await input.setValue(text);
    await input.trigger("keydown", { key: "Enter" });
  }

  it("shows a checkbox row for each item as it's added, before the quest is created", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="new-quest-checklist-toggle"]').trigger("click");
    await addChecklistDraftItem(wrapper, "headphones");
    await addChecklistDraftItem(wrapper, "key fob");

    const rows = wrapper.findAll('[data-testid="new-quest-checklist"] > div');
    // Two item rows plus the trailing "add item" row.
    expect(rows.length).toBe(3);
    expect(wrapper.text()).toContain("headphones");
    expect(wrapper.text()).toContain("key fob");
  });

  it("checking an item while composing shows it as checked before submit", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="new-quest-checklist-toggle"]').trigger("click");
    await addChecklistDraftItem(wrapper, "headphones");

    const checkbox = wrapper.find('[data-testid="new-quest-checklist"] button[aria-label="Check item"]');
    await checkbox.trigger("click");

    expect(wrapper.find('[aria-label="Uncheck item"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="new-quest-checklist"]').text()).toContain("headphones");
  });

  it("submits composed checklist items, including their checked state and one left in the input", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Go to office");
    await wrapper.find('[data-testid="new-quest-checklist-toggle"]').trigger("click");
    await addChecklistDraftItem(wrapper, "headphones");
    await wrapper.find('[data-testid="new-quest-checklist"] button[aria-label="Check item"]').trigger("click");
    // Left in the input, not confirmed with Enter — must still be captured on submit.
    await wrapper.find('[data-testid="new-quest-checklist-item-input"]').setValue("lunch");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledTimes(1);
    const call = createQuest.mock.calls[0][0];
    expect(call.title).toBe("Go to office");
    expect(call.is_checklist).toBe(true);
    expect(call.description).toMatch(
      /^- \[x\] headphones <!--k=.+-->\n- \[ \] lunch <!--k=.+-->$/,
    );
  });

  it("preserves checklist mode for an empty checklist draft", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Pack later");
    await wrapper.find('[data-testid="new-quest-checklist-toggle"]').trigger("click");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledWith(expect.objectContaining({
      title: "Pack later",
      description: null,
      is_checklist: true,
    }));
  });

  it("allows non-empty metadata drafts to collapse", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="new-quest-expand"]').trigger("click");
    await wrapper.find('[data-testid="new-quest-description"]').setValue("Keep this hidden while collapsed");

    await wrapper.find('[data-testid="new-quest-expand"]').trigger("click");

    expect(wrapper.find('[data-testid="new-quest-description"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="new-quest-expand"]').text()).toContain("More");
    expect(wrapper.find('[data-testid="new-quest-expand"]').attributes("aria-expanded")).toBe("false");

    await wrapper.find('[data-testid="new-quest-expand"]').trigger("click");

    expect((wrapper.find('[data-testid="new-quest-description"]').element as HTMLTextAreaElement).value).toBe(
      "Keep this hidden while collapsed",
    );
  });

  it("drops reminder time when no due date is selected", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: {
            template: '<button data-testid="stub-reminder-save" @click="$emit(\'save\', payload)">save reminder</button>',
            props: ["quest"],
            emits: ["save", "close"],
            data: () => ({ payload: { due: null, due_time: "14:30", repeat_rule: null } }),
          },
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Do not keep invisible reminder time");
    await wrapper.find('[data-testid="new-quest-reminder"]').trigger("click");
    await wrapper.find('[data-testid="stub-reminder-save"]').trigger("click");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledWith({
      title: "Do not keep invisible reminder time",
      description: null,
      is_checklist: false,
      space_id: "1",
      due: null,
      due_time: null,
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

  it("prevents duplicate creates while submit is pending", async () => {
    let resolveCreate: () => void = () => {};
    createQuest.mockReturnValue(new Promise<void>((resolve) => {
      resolveCreate = resolve;
    }));
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Avoid duplicates");
    await wrapper.find("form").trigger("submit");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledTimes(1);
    expect(wrapper.find('[data-testid="chat-submit"]').attributes("disabled")).toBeDefined();

    resolveCreate();
    await nextTick();
  });

  it("disables reminder controls while submit is pending", async () => {
    let resolveCreate: () => void = () => {};
    createQuest.mockReturnValue(new Promise<void>((resolve) => {
      resolveCreate = resolve;
    }));
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

    await wrapper.find('[data-testid="chat-input"]').setValue("Keep reminder stable");
    await wrapper.find('[data-testid="new-quest-reminder"]').trigger("click");
    await wrapper.find('[data-testid="stub-reminder-save"]').trigger("click");
    await wrapper.find("form").trigger("submit");
    await nextTick();

    expect(wrapper.find('[data-testid="new-quest-reminder"]').attributes("disabled")).toBeDefined();
    expect(wrapper.find('[data-testid="new-quest-clear-reminder"]').attributes("disabled")).toBeDefined();

    await wrapper.find('[data-testid="new-quest-reminder"]').trigger("click");

    expect(wrapper.find('[data-testid="stub-reminder-save"]').exists()).toBe(false);

    resolveCreate();
    await nextTick();
  });

  it("keeps an empty draft space aligned with the active filter", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    spaceStoreState.selectedSpaceId = "2";
    await nextTick();
    await wrapper.find('[data-testid="chat-input"]').setValue("Create in filtered space");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledWith({
      title: "Create in filtered space",
      description: null,
      is_checklist: false,
      space_id: "2",
      due: null,
      due_time: null,
      repeat_rule: null,
    });
  });

  it("resyncs draft space after skipped filter changes when the draft becomes empty", async () => {
    const wrapper = mount(NewQuestForm, {
      global: {
        stubs: {
          ReminderMenu: true,
        },
      },
    });

    await wrapper.find('[data-testid="chat-input"]').setValue("Started in Personal");
    spaceStoreState.selectedSpaceId = "2";
    await nextTick();
    await wrapper.find('[data-testid="chat-input"]').setValue("");
    await nextTick();
    await wrapper.find('[data-testid="chat-input"]').setValue("Create in refreshed filter");
    await wrapper.find("form").trigger("submit");

    expect(createQuest).toHaveBeenCalledWith({
      title: "Create in refreshed filter",
      description: null,
      is_checklist: false,
      space_id: "2",
      due: null,
      due_time: null,
      repeat_rule: null,
    });
  });
});
