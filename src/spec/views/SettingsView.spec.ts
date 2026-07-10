import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SettingsView from "../../views/SettingsView.vue";
import { useSpaceStore } from "../../stores/space";
import { useDeviceStore } from "../../stores/device";

interface MockSpace {
  id: string;
  name: string;
  item_order: number;
  created_at: string;
}

interface MockPairedDevice {
  peer_device_id: string;
  display_name: string;
  paired_at: string;
  last_seen_at: string | null;
  pair_state: string;
}

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
  EllipsisVerticalIcon: { name: "EllipsisVerticalIcon", template: "<span />" },
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
  __esModule: true,
  default: { name: "AboutCard", template: "<section data-testid='about-card-stub' />" },
}));

jest.mock("../../components/SettingsView/ExportSpacesDialog.vue", () => ({
  __esModule: true,
  default: { name: "ExportSpacesDialog", template: "<section data-testid='export-spaces-dialog-stub' />" },
}));

jest.mock("../../components/SettingsView/ImportSpaceMappingDialog.vue", () => ({
  __esModule: true,
  default: { name: "ImportSpaceMappingDialog", template: "<section data-testid='import-space-mapping-dialog-stub' />" },
}));

jest.mock("../../components/SettingsView/MergeConflictDialog.vue", () => ({
  __esModule: true,
  default: { name: "MergeConflictDialog", template: "<section data-testid='merge-conflict-dialog-stub' />" },
}));

jest.mock("../../components/SettingsView/ThemeSelector.vue", () => ({
  __esModule: true,
  default: { name: "ThemeSelector", template: "<section data-testid='theme-selector-stub' />" },
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

function mockSettingsStores({
  spaces = [],
  pairedDevices = [],
  onlineDeviceIds = [],
}: {
  spaces?: MockSpace[];
  pairedDevices?: MockPairedDevice[];
  onlineDeviceIds?: string[];
} = {}) {
  const onlineIds = new Set(onlineDeviceIds);
  (useSpaceStore as unknown as jest.Mock).mockReturnValue({
    spaces,
    error: null,
    fetchSpaces: jest.fn().mockResolvedValue(undefined),
    createSpace: jest.fn().mockResolvedValue(undefined),
    updateSpace: jest.fn().mockResolvedValue(undefined),
    deleteSpace: jest.fn().mockResolvedValue(undefined),
  });
  (useDeviceStore as unknown as jest.Mock).mockReturnValue({
    pairedDevices,
    hydrate: jest.fn().mockResolvedValue(undefined),
    isDeviceOnline: (device: MockPairedDevice) => onlineIds.has(device.peer_device_id),
  });
}

describe("SettingsView search", () => {
  beforeEach(() => {
    mockSettingsStores({
      spaces: [
        { id: "1", name: "Personal", item_order: 0, created_at: "2026-01-01T00:00:00Z" },
        { id: "2", name: "Family", item_order: 1, created_at: "2026-01-01T00:00:00Z" },
        { id: "4", name: "Workshop", item_order: 2, created_at: "2026-01-01T00:00:00Z" },
      ],
      pairedDevices: [
        {
          peer_device_id: "peer-kitchen-uuid",
          display_name: "Kitchen iPad",
          paired_at: "2026-01-01T00:00:00Z",
          last_seen_at: null,
          pair_state: "paired",
        },
        {
          peer_device_id: "peer-hall-uuid",
          display_name: "Hall Phone",
          paired_at: "2026-01-01T00:00:00Z",
          last_seen_at: null,
          pair_state: "paired",
        },
      ],
      onlineDeviceIds: ["peer-kitchen-uuid"],
    });
    (invoke as jest.Mock).mockReset();
    (invoke as jest.Mock).mockResolvedValue(false);
  });

  it("filters settings rows by visible labels and restores the overview when cleared", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    const search = wrapper.find('[data-testid="settings-search-input"]');
    await search.setValue("family");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-spaces"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("Family");
    expect(wrapper.text()).not.toContain("Personal");
    expect(wrapper.find('[data-testid="settings-devices"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="settings-backup"]').exists()).toBe(false);

    await search.setValue("");
    await flushUi();

    expect(wrapper.text()).toContain("Personal");
    expect(wrapper.find('[data-testid="settings-devices"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="settings-backup"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="theme-selector-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="about-card-stub"]').exists()).toBe(true);
  });

  it("keeps section matches in place without showing unrelated sections", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("backup");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-backup"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("Export backup");
    expect(wrapper.text()).toContain("Import backup");
    expect(wrapper.find('[data-testid="settings-spaces"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="settings-devices"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="theme-selector-stub"]').exists()).toBe(false);
  });

  it("matches device display names and presence labels without searching UUIDs", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    const search = wrapper.find('[data-testid="settings-search-input"]');
    await search.setValue("online");
    await flushUi();

    expect(wrapper.findAll('[data-testid="paired-device-row"]')).toHaveLength(1);
    expect(wrapper.text()).toContain("Kitchen iPad");
    expect(wrapper.text()).toContain("Online");
    expect(wrapper.text()).not.toContain("Hall Phone");
    expect(wrapper.find('[data-testid="add-device-link"]').exists()).toBe(false);

    await search.setValue("peer-hall-uuid");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-search-empty"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("No settings found");
    expect(wrapper.text()).not.toContain("Hall Phone");
  });
});

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
    (invoke as jest.Mock)
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(false);

    const wrapper = mountSettingsView();
    await flushUi();

    const toggle = wrapper.find('[data-testid="automatic-updates-toggle"]');
    expect(invoke).toHaveBeenCalledWith("startup_auto_update_supported");
    expect(invoke).toHaveBeenCalledWith("get_auto_update_enabled");
    expect(wrapper.text()).toContain("Automatic updates");
    expect(wrapper.text()).toContain("When this is off, Fini will not install updates automatically on the next restart.");
    expect((toggle.element as HTMLInputElement).checked).toBe(false);
  });

  it("persists automatic updates toggle changes", async () => {
    (invoke as jest.Mock)
      .mockResolvedValueOnce(true)
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

  it("hides automatic updates settings when startup updates are unsupported", async () => {
    (invoke as jest.Mock).mockResolvedValueOnce(false);

    const wrapper = mountSettingsView();
    await flushUi();

    expect(wrapper.find('[data-testid="settings-updates"]').exists()).toBe(false);
    expect(invoke).toHaveBeenCalledWith("startup_auto_update_supported");
    expect(invoke).not.toHaveBeenCalledWith("get_auto_update_enabled");
  });
});
