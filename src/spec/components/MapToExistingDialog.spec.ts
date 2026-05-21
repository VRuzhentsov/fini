import { mount } from "@vue/test-utils";
import MapToExistingDialog from "../../components/MapToExistingDialog.vue";

jest.mock("../../stores/space", () => ({
  SPACE_COLOR_CLASS: {},
  BUILTIN_SPACE_IDS: ["1", "2", "3"],
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
}));

jest.mock("../../utils/shortUuid", () => ({
  shortUuid: (id: string) => id.slice(0, 8),
}));

const spaces = [
  { id: "custom-1", name: "Work", item_order: 0, created_at: "" },
  { id: "custom-2", name: "Personal", item_order: 1, created_at: "" },
];

describe("MapToExistingDialog", () => {
  it("renders space list as radio rows", () => {
    const wrapper = mount(MapToExistingDialog, {
      props: { context: { kind: "backup-space", name: "Incoming" }, spaces },
      global: { stubs: { Teleport: true } },
    });

    const rows = wrapper.findAll('[role="option"]');
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("Work");
    expect(rows[1].text()).toContain("Personal");
  });

  it("Map button is disabled until a space is selected", () => {
    const wrapper = mount(MapToExistingDialog, {
      props: { context: { kind: "backup-space", name: "Incoming" }, spaces },
      global: { stubs: { Teleport: true } },
    });

    const mapBtn = wrapper.findAll("button").find((b) => b.text().startsWith("Map"))!;
    expect(mapBtn.attributes("disabled")).toBeDefined();
  });

  it("emits confirm with selected space id when Map is clicked", async () => {
    const wrapper = mount(MapToExistingDialog, {
      props: { context: { kind: "backup-space", name: "Incoming" }, spaces },
      global: { stubs: { Teleport: true } },
    });

    await wrapper.findAll('[role="option"]')[0].trigger("click");
    const mapBtn = wrapper.findAll("button").find((b) => b.text().startsWith("Map"))!;
    expect(mapBtn.attributes("disabled")).toBeUndefined();
    await mapBtn.trigger("click");
    expect(wrapper.emitted("confirm")?.[0]).toEqual(["custom-1"]);
  });

  it("uses peer-space copy when context.kind is peer-space", () => {
    const wrapper = mount(MapToExistingDialog, {
      props: { context: { kind: "peer-space", name: "Remote" }, spaces },
      global: { stubs: { Teleport: true } },
    });

    expect(wrapper.text()).toContain("Remote");
    expect(wrapper.text()).toContain("sync");
  });

  it("shows empty message when no spaces", () => {
    const wrapper = mount(MapToExistingDialog, {
      props: { context: { kind: "backup-space", name: "Incoming" }, spaces: [] },
      global: { stubs: { Teleport: true } },
    });

    expect(wrapper.text()).toContain("No local custom spaces");
  });
});
