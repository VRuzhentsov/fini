import { mount } from "@vue/test-utils";
import QuestEditor from "../../components/QuestEditor.vue";
import type { Quest } from "../../stores/quest";

function baseQuest(overrides: Partial<Quest> = {}): Quest {
  return {
    id: "q1",
    space_id: "1",
    title: "Go to office",
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
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    series_id: null,
    period_key: null,
    is_checklist: false,
    ...overrides,
  };
}

const defaultProps = {
  spaceName: "Personal",
  priorityColor: "oklch(var(--color-base-content)/0.3)",
  priorityLabel: "None",
};

describe("QuestEditor checklist rendering", () => {
  it("renders the prose textarea, not the checklist section, for a non-checklist quest", () => {
    const wrapper = mount(QuestEditor, {
      props: { ...defaultProps, quest: baseQuest({ is_checklist: false, description: "notes" }) },
    });

    expect(wrapper.find(".quest-editor-checklist").exists()).toBe(false);
    expect(wrapper.find(".quest-editor-desc").exists()).toBe(true);
  });

  it("renders the checklist section, not the textarea, for a checklist quest", () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          is_checklist: true,
          description: "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->",
        }),
      },
    });

    expect(wrapper.find(".quest-editor-checklist").exists()).toBe(true);
    expect(wrapper.find("textarea.quest-editor-desc").exists()).toBe(false);
    expect(wrapper.find(".quest-editor-checklist-count").text()).toBe("1/2");
  });

  it("emits toggleChecklistItem with the item id and inverted checked state", async () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          is_checklist: true,
          description: "- [ ] headphones <!--k=a1-->",
        }),
      },
    });

    await wrapper.find('button[aria-label="Check item"]').trigger("click");

    expect(wrapper.emitted("toggleChecklistItem")).toEqual([["a1", true]]);
  });

  it("emits addChecklistItem when submitting the add-item input", async () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({ is_checklist: true, description: "" }),
      },
    });

    const input = wrapper.find('input[placeholder="Add item"]');
    await input.setValue("lunch");
    await input.trigger("keydown", { key: "Enter" });

    expect(wrapper.emitted("addChecklistItem")).toEqual([["lunch"]]);
  });

  it("keeps active checklist item text readonly until the item is held", async () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          is_checklist: true,
          description: "- [ ] headpones <!--k=a1-->",
        }),
      },
    });

    // Per the Fini App Refresh design, item text is a read-only label until held (#128) —
    // instant editability was the app's own earlier, non-canonical behavior.
    expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(false);
    expect(wrapper.find(".checklist-item-text").text()).toBe("headpones");
  });

  it("reveals an editable input after holding an active checklist item, and emits the edit on blur", async () => {
    jest.useFakeTimers();
    try {
      const wrapper = mount(QuestEditor, {
        props: {
          ...defaultProps,
          quest: baseQuest({
            is_checklist: true,
            description: "- [ ] headpones <!--k=a1-->",
          }),
        },
      });

      await wrapper.find(".checklist-item-text").trigger("pointerdown");
      jest.advanceTimersByTime(600);
      await Promise.resolve();
      await Promise.resolve();

      const input = wrapper.find<HTMLInputElement>('input[aria-label="Checklist item text"]');
      expect(input.exists()).toBe(true);
      await input.setValue("headphones");
      await input.trigger("blur");

      expect(wrapper.emitted("editChecklistItemText")).toEqual([["a1", "headphones"]]);
    } finally {
      jest.useRealTimers();
    }
  });

  it("does not reveal an editable input on a quick (non-held) tap of an active checklist item", async () => {
    jest.useFakeTimers();
    try {
      const wrapper = mount(QuestEditor, {
        props: {
          ...defaultProps,
          quest: baseQuest({
            is_checklist: true,
            description: "- [ ] headpones <!--k=a1-->",
          }),
        },
      });

      await wrapper.find(".checklist-item-text").trigger("pointerdown");
      jest.advanceTimersByTime(200);
      await wrapper.find(".checklist-item-text").trigger("pointerup");
      jest.advanceTimersByTime(600);
      await Promise.resolve();

      expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(false);
    } finally {
      jest.useRealTimers();
    }
  });

  it("keeps completed checklist item text readonly and not hold-editable", () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          status: "completed",
          is_checklist: true,
          description: "- [x] headphones <!--k=a1-->",
        }),
      },
    });

    expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(false);
    expect(wrapper.find(".checklist-item-text").text()).toBe("headphones");
  });

  it("disables checklist item toggles for completed checklist quests", async () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          status: "completed",
          is_checklist: true,
          description: "- [x] headphones <!--k=a1-->",
        }),
      },
    });

    const checkbox = wrapper.find<HTMLButtonElement>('button[aria-label="Uncheck item"]');

    expect(checkbox.element.disabled).toBe(true);
    await checkbox.trigger("click");
    expect(wrapper.emitted("toggleChecklistItem")).toBeUndefined();
  });

  it("hides remove buttons and the add-item row for a completed (readonly) checklist quest", () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          status: "completed",
          is_checklist: true,
          description: "- [x] headphones <!--k=a1-->",
        }),
      },
    });

    expect(wrapper.find('button[aria-label="Remove item"]').exists()).toBe(false);
    expect(wrapper.find('input[placeholder="Add item"]').exists()).toBe(false);
  });

  it("shows the audit trail only when checklistActivity is provided on a checklist quest", () => {
    const withActivity = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          status: "completed",
          is_checklist: true,
          description: "- [x] headphones <!--k=a1-->",
        }),
        checklistActivity: [
          {
            id: "act-1",
            quest_id: "q1",
            kind: "completed_snapshot",
            detail: "Checklist at completion: 1/1 checked",
            created_at: "2026-01-01T00:00:00Z",
            origin_device_id: null,
          },
        ],
      },
    });
    expect(withActivity.find(".quest-editor-checklist-audit").exists()).toBe(true);
    expect(withActivity.find(".quest-editor-checklist-audit-detail").text()).toBe(
      "Checklist at completion: 1/1 checked",
    );

    const withoutActivity = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          status: "completed",
          is_checklist: true,
          description: "- [x] headphones <!--k=a1-->",
        }),
      },
    });
    expect(withoutActivity.find(".quest-editor-checklist-audit").exists()).toBe(false);
  });
});
