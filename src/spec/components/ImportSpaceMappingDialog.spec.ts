import { mount } from "@vue/test-utils";
import ImportSpaceMappingDialog from "../../components/SettingsView/ImportSpaceMappingDialog.vue";

jest.mock("../../stores/space", () => ({
  SPACE_COLOR_CLASS: {},
  BUILTIN_SPACE_IDS: ["1", "2", "3"],
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
}));

jest.mock("../../utils/shortUuid", () => ({
  shortUuid: (id: string) => id.slice(0, 8),
}));

const incoming = { backup_space_id: "abc-123", backup_space_name: "Work" };
const localSpaces = [{ id: "custom-1", name: "My Space", item_order: 0, created_at: "" }];

describe("ImportSpaceMappingDialog", () => {
  it("renders 3 footer buttons: Cancel, Map to existing, Create", () => {
    const wrapper = mount(ImportSpaceMappingDialog, {
      props: { incoming, localSpaces, index: 0, total: 1 },
      global: { stubs: { Teleport: true, MapToExistingDialog: true } },
    });

    const buttons = wrapper.findAll("button");
    const labels = buttons.map((b) => b.text());
    expect(labels).toContain("Cancel");
    expect(labels).toContain("Map to existing");
    expect(labels).toContain("Create");
  });

  it("does not show counter when total is 1", () => {
    const wrapper = mount(ImportSpaceMappingDialog, {
      props: { incoming, localSpaces, index: 0, total: 1 },
      global: { stubs: { Teleport: true, MapToExistingDialog: true } },
    });

    expect(wrapper.find('[data-testid="mapping-counter"]').exists()).toBe(false);
  });

  it("shows counter when total > 1", () => {
    const wrapper = mount(ImportSpaceMappingDialog, {
      props: { incoming, localSpaces, index: 0, total: 3 },
      global: { stubs: { Teleport: true, MapToExistingDialog: true } },
    });

    expect(wrapper.find('[data-testid="mapping-counter"]').text()).toBe("1/3");
  });

  it("shows choice cards with Create and Map to existing descriptions", () => {
    const wrapper = mount(ImportSpaceMappingDialog, {
      props: { incoming, localSpaces, index: 0, total: 1 },
      global: { stubs: { Teleport: true, MapToExistingDialog: true } },
    });

    const text = wrapper.text();
    expect(text).toContain("Create");
    expect(text).toContain("Map to existing");
    expect(text).toContain("Work");
  });

  it("emits resolve with create_new when Create is clicked", async () => {
    const wrapper = mount(ImportSpaceMappingDialog, {
      props: { incoming, localSpaces, index: 0, total: 1 },
      global: { stubs: { Teleport: true, MapToExistingDialog: true } },
    });

    await wrapper.findAll("button").find((b) => b.text() === "Create")!.trigger("click");
    expect(wrapper.emitted("resolve")?.[0]).toEqual([{ mode: "create_new" }]);
  });
});
