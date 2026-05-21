import { mount } from "@vue/test-utils";
import MergeConflictDialog from "../../components/SettingsView/MergeConflictDialog.vue";

const conflicts = [
  {
    entity_type: "quest",
    id: "quest-1",
    title: "Quest one",
    local_summary: "Local quest",
    backup_summary: "Backup quest",
    local: { title: "Local quest" },
    backup: { title: "Backup quest" },
  },
  {
    entity_type: "quest_series",
    id: "series-1",
    title: "Series one",
    local_summary: "Local series",
    backup_summary: "Backup series",
    local: { title: "Local series" },
    backup: { title: "Backup series" },
  },
];

describe("MergeConflictDialog", () => {
  it("renders short counter and requires every conflict to be resolved", async () => {
    const wrapper = mount(MergeConflictDialog, {
      props: { conflicts },
      global: { stubs: { Teleport: true } },
    });

    expect(wrapper.find('[data-testid="merge-conflict-counter"]').text()).toBe("0/2");

    const applyBtn = wrapper.findAll("button").find((b) => b.text().startsWith("Apply"));
    expect(applyBtn).toBeDefined();
    expect(applyBtn!.attributes("disabled")).toBeDefined();

    // choose "Use local" for conflict 1 (btn-outline when unselected)
    await wrapper.get("button.btn-outline").trigger("click");
    expect(wrapper.find('[data-testid="merge-conflict-counter"]').text()).toBe("1/2");

    // navigate to conflict 2 via › button
    const nextBtn = wrapper.findAll("button").find((b) => b.attributes("aria-label") === "Next conflict");
    expect(nextBtn).toBeDefined();
    await nextBtn!.trigger("click");

    // choose "Use backup" for conflict 2
    await wrapper.findAll("button").find((b) => b.text() === "Use backup")!.trigger("click");
    expect(wrapper.find('[data-testid="merge-conflict-counter"]').text()).toBe("2/2");

    const apply = wrapper.findAll("button").find((b) => b.text().startsWith("Apply"))!;
    expect(apply.attributes("disabled")).toBeUndefined();
    await apply.trigger("click");

    expect(wrapper.emitted("apply")?.[0][0]).toEqual([
      { entity_type: "quest", id: "quest-1", resolution: "local" },
      { entity_type: "quest_series", id: "series-1", resolution: "backup" },
    ]);
  });

  it("shows kind-pill for quest and quest_series", () => {
    const wrapper = mount(MergeConflictDialog, {
      props: { conflicts },
      global: { stubs: { Teleport: true } },
    });

    const pills = wrapper.findAll(".badge");
    expect(pills.length).toBeGreaterThan(0);
    expect(pills[0].text()).toBe("Quest");
  });
});
