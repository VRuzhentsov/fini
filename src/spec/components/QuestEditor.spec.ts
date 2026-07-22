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

    await wrapper.find(".quest-editor-checklist-box").trigger("click");

    expect(wrapper.emitted("toggleChecklistItem")).toEqual([["a1", true]]);
  });

  it("emits addChecklistItem when submitting the add-item input", async () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({ is_checklist: true, description: "" }),
      },
    });

    const input = wrapper.find(".quest-editor-checklist-add input");
    await input.setValue("lunch");
    await input.trigger("keydown", { key: "Enter" });

    expect(wrapper.emitted("addChecklistItem")).toEqual([["lunch"]]);
  });

  it("lets active checklist item text edits emit the existing item id and new text", async () => {
    const wrapper = mount(QuestEditor, {
      props: {
        ...defaultProps,
        quest: baseQuest({
          is_checklist: true,
          description: "- [ ] headpones <!--k=a1-->",
        }),
      },
    });

    const input = wrapper.find(".quest-editor-checklist-text-input");
    await input.setValue("headphones");
    await input.trigger("blur");

    expect(wrapper.emitted("editChecklistItemText")).toEqual([["a1", "headphones"]]);
  });

  it("keeps completed checklist item text readonly", () => {
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

    expect(wrapper.find(".quest-editor-checklist-text-input").exists()).toBe(false);
    expect(wrapper.find(".quest-editor-checklist-text").text()).toBe("headphones");
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

    const checkbox = wrapper.find<HTMLButtonElement>(".quest-editor-checklist-box");

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

    expect(wrapper.find(".quest-editor-checklist-remove").exists()).toBe(false);
    expect(wrapper.find(".quest-editor-checklist-add").exists()).toBe(false);
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
