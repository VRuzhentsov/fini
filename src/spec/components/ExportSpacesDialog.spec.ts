import { mount } from "@vue/test-utils";
import ExportSpacesDialog from "../../components/SettingsView/ExportSpacesDialog.vue";
import { useSpaceStore } from "../../stores/space";
import { useQuestStore } from "../../stores/quest";
import { useBackupStore } from "../../stores/backup";

jest.mock("@tauri-apps/plugin-dialog", () => ({
  save: jest.fn(),
}));

jest.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir: jest.fn(),
}));

jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  SPACE_COLOR_CLASS: {},
  BUILTIN_SPACE_IDS: ["1", "2", "3"],
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
}));

jest.mock("../../stores/quest", () => ({
  useQuestStore: jest.fn(),
}));

jest.mock("../../stores/backup", () => ({
  useBackupStore: jest.fn(),
}));

jest.mock("../../composables/useToast", () => ({
  useToast: () => ({ show: jest.fn(), error: jest.fn() }),
}));

describe("ExportSpacesDialog", () => {
  beforeEach(() => {
    (useSpaceStore as unknown as jest.Mock).mockReturnValue({
      spaces: [
        { id: "1", name: "Personal" },
        { id: "custom", name: "Custom" },
      ],
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
    });
    (useQuestStore as unknown as jest.Mock).mockReturnValue({
      quests: [],
      fetchQuests: jest.fn().mockResolvedValue(undefined),
    });
    (useBackupStore as unknown as jest.Mock).mockReturnValue({
      exportBackup: jest.fn().mockResolvedValue({ path: "/tmp/backup.zip" }),
    });
  });

  it("starts with no spaces selected and disables Export", async () => {
    const wrapper = mount(ExportSpacesDialog, {
      global: { stubs: { Teleport: true } },
    });

    const checkboxes = wrapper.findAll('input[type="checkbox"]');
    expect(checkboxes).toHaveLength(2);
    expect(checkboxes.every((c) => (c.element as HTMLInputElement).checked === false)).toBe(true);

    const exportBtn = wrapper.findAll("button").find((b) => b.text().startsWith("Export"))!;
    expect(exportBtn.attributes("disabled")).toBeDefined();
  });

  it("enables Export when at least one space is checked", async () => {
    const wrapper = mount(ExportSpacesDialog, {
      global: { stubs: { Teleport: true } },
    });

    await wrapper.findAll('input[type="checkbox"]')[0].setValue(true);

    const exportBtn = wrapper.findAll("button").find((b) => b.text().startsWith("Export"))!;
    expect(exportBtn.attributes("disabled")).toBeUndefined();
  });

  it("select-all toggles all spaces", async () => {
    const wrapper = mount(ExportSpacesDialog, {
      global: { stubs: { Teleport: true } },
    });

    const selectAllBtn = wrapper.findAll("button").find((b) => b.text() === "Select all")!;
    await selectAllBtn.trigger("click");

    const checkboxes = wrapper.findAll('input[type="checkbox"]');
    expect(checkboxes.every((c) => (c.element as HTMLInputElement).checked === true)).toBe(true);
  });
});
