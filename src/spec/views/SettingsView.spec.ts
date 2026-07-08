import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SettingsView from "../../views/SettingsView.vue";
import { useSpaceStore } from "../../stores/space";
import { useDeviceStore } from "../../stores/device";

jest.mock("@tauri-apps/api/core", () => ({
  invoke: jest.fn(),
}));

jest.mock("../../package.json", () => ({
  __esModule: true,
  default: { version: "0.1.41" },
  version: "0.1.41",
}), { virtual: true });

jest.mock("../../../package.json", () => ({
  __esModule: true,
  default: { version: "0.1.41" },
  version: "0.1.41",
}));

jest.mock("@heroicons/vue/24/outline", () => ({
  PencilIcon: { name: "PencilIcon", template: "<span />" },
  TrashIcon: { name: "TrashIcon", template: "<span />" },
}));

jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
}));

jest.mock("../../stores/device", () => ({
  useDeviceStore: jest.fn(),
}));

jest.mock("../../composables/useContextMenu", () => ({
  useContextMenu: () => ({ open: jest.fn() }),
}));

jest.mock("../../composables/useBackupImport", () => ({
  useBackupImport: () => ({
    startImport: jest.fn(),
    cancelImport: jest.fn(),
    confirmMapping: jest.fn(),
    applyImport: jest.fn(),
    error: { value: null },
    activeMapping: { value: null },
    selectableSpaces: { value: [] },
    mappingIndex: { value: 0 },
    totalMappings: { value: 0 },
    showConflicts: { value: false },
    conflicts: { value: [] },
  }),
}));

jest.mock("../../components/SettingsView/AboutCard.vue", () => ({
  name: "AboutCard",
  template: "<section data-testid='about-card-stub' />",
}));

jest.mock("../../components/SettingsView/ExportSpacesDialog.vue", () => ({
  name: "ExportSpacesDialog",
  template: "<section data-testid='export-spaces-dialog-stub' />",
}));

jest.mock("../../components/SettingsView/ImportSpaceMappingDialog.vue", () => ({
  name: "ImportSpaceMappingDialog",
  template: "<section data-testid='import-space-mapping-dialog-stub' />",
}));

jest.mock("../../components/SettingsView/MergeConflictDialog.vue", () => ({
  name: "MergeConflictDialog",
  template: "<section data-testid='merge-conflict-dialog-stub' />",
}));

jest.mock("../../components/SettingsView/ThemeSelector.vue", () => ({
  name: "ThemeSelector",
  template: "<section data-testid='theme-selector-stub' />",
}));

async function flushUi() {
  for (let i = 0; i < 4; i += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

function mountSettingsView() {
  return mount(SettingsView, {
    global: {
      stubs: {
        "router-link": { template: "<a><slot /></a>" },
        ThemeSelector: { template: "<section data-testid='theme-selector-stub' />" },
        AboutCard: { template: "<section data-testid='about-card-stub' />", props: ["version", "sourceUrl"] },
        ExportSpacesDialog: true,
        ImportSpaceMappingDialog: true,
        MergeConflictDialog: true,
      },
    },
  });
}

describe("SettingsView automatic updates", () => {
  beforeEach(() => {
    (useSpaceStore as unknown as jest.Mock).mockReturnValue({
      spaces: [],
      error: null,
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
      createSpace: jest.fn().mockResolvedValue(undefined),
      updateSpace: jest.fn().mockResolvedValue(undefined),
      deleteSpace: jest.fn().mockResolvedValue(undefined),
    });
    (useDeviceStore as unknown as jest.Mock).mockReturnValue({
      pairedDevices: [],
      hydrate: jest.fn().mockResolvedValue(undefined),
      isDeviceOnline: jest.fn().mockReturnValue(false),
    });
    (invoke as jest.Mock).mockReset();
  });

  it("renders the automatic updates toggle from persisted settings", async () => {
    (invoke as jest.Mock).mockResolvedValueOnce(false);

    const wrapper = mountSettingsView();
    await flushUi();

    const toggle = wrapper.find('[data-testid="automatic-updates-toggle"]');
    expect(invoke).toHaveBeenCalledWith("get_auto_update_enabled");
    expect(wrapper.text()).toContain("Automatic updates");
    expect(wrapper.text()).toContain("When this is off, Fini will not install updates automatically on the next restart.");
    expect((toggle.element as HTMLInputElement).checked).toBe(false);
  });

  it("persists automatic updates toggle changes", async () => {
    (invoke as jest.Mock)
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(false);

    const wrapper = mountSettingsView();
    await flushUi();

    const toggle = wrapper.find('[data-testid="automatic-updates-toggle"]');
    expect((toggle.element as HTMLInputElement).checked).toBe(true);

    await toggle.setValue(false);
    await flushUi();

    expect(invoke).toHaveBeenLastCalledWith("set_auto_update_enabled", { enabled: false });
    expect((toggle.element as HTMLInputElement).checked).toBe(false);
  });
});
