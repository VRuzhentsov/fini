import { mount } from "@vue/test-utils";
import ChecklistEditor from "../../components/ChecklistEditor.vue";
import type { ChecklistItem } from "../../utils/checklist";

function items(): ChecklistItem[] {
  return [
    { id: "a1", text: "headphones", checked: false },
    { id: "a2", text: "key fob", checked: true },
  ];
}

describe("ChecklistEditor", () => {
  describe("draft mode (composer)", () => {
    it("renders every item's text as an always-editable input", () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "draft" } });
      const inputs = wrapper.findAll('input[aria-label="Checklist item text"]');
      expect(inputs.map((i) => (i.element as HTMLInputElement).value)).toEqual([
        "headphones",
        "key fob",
      ]);
    });

    it("emits toggle-item standalone, without also emitting update:items", async () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "draft" } });
      await wrapper.find('button[aria-label="Check item"]').trigger("click");

      expect(wrapper.emitted("toggle-item")).toEqual([["a1", true]]);
      expect(wrapper.emitted("update:items")).toBeUndefined();
    });

    it("emits add-item and update:items with the appended item on Enter", async () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "draft" } });
      const input = wrapper.find('input[placeholder="Add item"]');
      await input.setValue("lunch");
      await input.trigger("keydown", { key: "Enter" });

      expect(wrapper.emitted("add-item")).toEqual([["lunch"]]);
      const updateEvents = wrapper.emitted("update:items") as ChecklistItem[][][];
      expect(updateEvents[0][0].map((it) => it.text)).toEqual(["headphones", "key fob", "lunch"]);
    });

    it("emits edit-item-text and update:items on blur when text changes", async () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "draft" } });
      const input = wrapper.findAll('input[aria-label="Checklist item text"]')[0];
      await input.setValue("over-ear headphones");
      await input.trigger("blur");

      expect(wrapper.emitted("edit-item-text")).toEqual([["a1", "over-ear headphones"]]);
      const updateEvents = wrapper.emitted("update:items") as ChecklistItem[][][];
      expect(updateEvents[0][0][0].text).toBe("over-ear headphones");
    });

    it("emits remove-item and update:items with the item filtered out", async () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "draft" } });
      await wrapper.findAll('button[aria-label="Remove item"]')[0].trigger("click");

      expect(wrapper.emitted("remove-item")).toEqual([["a1"]]);
      const updateEvents = wrapper.emitted("update:items") as ChecklistItem[][][];
      expect(updateEvents[0][0].map((it) => it.id)).toEqual(["a2"]);
    });

    it("flushPendingItem (exposed) commits text left in the add-item input", async () => {
      const wrapper = mount(ChecklistEditor, { props: { items: [], mode: "draft" } });
      await wrapper.find('input[placeholder="Add item"]').setValue("lunch");

      (wrapper.vm as unknown as { flushPendingItem: () => void }).flushPendingItem();
      await wrapper.vm.$nextTick();

      expect(wrapper.emitted("add-item")).toEqual([["lunch"]]);
    });
  });

  describe("active mode (existing quest, editable)", () => {
    it("renders item text as a read-only label, not an input", () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "active" } });
      expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(false);
      expect(wrapper.findAll(".checklist-item-text")[0].text()).toBe("headphones");
    });

    it("reveals an editable input only after holding an item", async () => {
      jest.useFakeTimers();
      try {
        const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "active" } });
        await wrapper.findAll(".checklist-item-text")[0].trigger("pointerdown");
        jest.advanceTimersByTime(600);
        await Promise.resolve();
        await Promise.resolve();

        expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(true);
      } finally {
        jest.useRealTimers();
      }
    });

    it("supports add and remove", async () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "active" } });
      expect(wrapper.find('input[placeholder="Add item"]').exists()).toBe(true);
      expect(wrapper.findAll('button[aria-label="Remove item"]').length).toBe(2);
    });
  });

  describe("compact mode (Focus hero card)", () => {
    it("has no add-item input or remove buttons", () => {
      const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "compact" } });
      expect(wrapper.find('input[placeholder="Add item"]').exists()).toBe(false);
      expect(wrapper.find('button[aria-label="Remove item"]').exists()).toBe(false);
    });

    it("still toggles and still supports hold-to-edit", async () => {
      jest.useFakeTimers();
      try {
        const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "compact" } });
        await wrapper.find('button[aria-label="Check item"]').trigger("click");
        expect(wrapper.emitted("toggle-item")).toEqual([["a1", true]]);

        await wrapper.findAll(".checklist-item-text")[0].trigger("pointerdown");
        jest.advanceTimersByTime(600);
        await Promise.resolve();
        await Promise.resolve();
        expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(true);
      } finally {
        jest.useRealTimers();
      }
    });
  });

  describe("readonly mode (completed/abandoned snapshot)", () => {
    it("disables toggles and has no add/remove/edit affordances", async () => {
      jest.useFakeTimers();
      try {
        const wrapper = mount(ChecklistEditor, { props: { items: items(), mode: "readonly" } });
        const checkbox = wrapper.find<HTMLButtonElement>("button");
        expect(checkbox.element.disabled).toBe(true);

        expect(wrapper.find('input[placeholder="Add item"]').exists()).toBe(false);
        expect(wrapper.find('button[aria-label="Remove item"]').exists()).toBe(false);

        await wrapper.findAll(".checklist-item-text")[0].trigger("pointerdown");
        jest.advanceTimersByTime(600);
        await Promise.resolve();
        expect(wrapper.find('input[aria-label="Checklist item text"]').exists()).toBe(false);
      } finally {
        jest.useRealTimers();
      }
    });
  });

  it("respects the disabled prop by disabling toggle/add/remove controls", () => {
    const wrapper = mount(ChecklistEditor, {
      props: { items: items(), mode: "active", disabled: true },
    });
    expect(wrapper.find<HTMLButtonElement>('button[aria-label="Uncheck item"]').element.disabled).toBe(
      true,
    );
    expect(
      wrapper.find<HTMLInputElement>('input[placeholder="Add item"]').element.disabled,
    ).toBe(true);
  });
});
