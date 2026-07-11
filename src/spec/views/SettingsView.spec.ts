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

jest.mock("../../components/SettingsView/SpacesSettingsSection.vue", () => ({
  __esModule: true,
  default: { name: "SpacesSettingsSection", template: "<section data-testid='spaces-settings-section-stub' />" },
}), { virtual: true });

jest.mock("../../components/SettingsView/DevicesSettingsSection.vue", () => ({
  __esModule: true,
  default: { name: "DevicesSettingsSection", template: "<section data-testid='devices-settings-section-stub' />" },
}), { virtual: true });

jest.mock("../../components/SettingsView/BackupSettingsSection.vue", () => ({
  __esModule: true,
  default: { name: "BackupSettingsSection", template: "<section data-testid='backup-settings-section-stub' />" },
}), { virtual: true });

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
        "router-link": { props: ["to"], template: "<a :data-to='to'><slot /></a>" },
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

  it("uses the full available width for the Settings search field", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    const search = wrapper.find('[data-testid="settings-search-input"]');
    expect(search.classes()).toContain("w-full");
    expect(search.attributes("placeholder")).toBe("Search settings");
    expect(search.element.parentElement?.textContent).toBe("");
  });

  it("renders overview sections from the dynamic section registry", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    expect(wrapper.find('[data-testid="spaces-settings-section-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="devices-settings-section-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="backup-settings-section-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="theme-selector-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="about-card-stub"]').exists()).toBe(true);
  });

  it("shows Android-style search results grouped by settings section and restores the overview when cleared", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    const search = wrapper.find('[data-testid="settings-search-input"]');
    await search.setValue("family");
    await flushUi();

    const results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Spaces");
    expect(results.text()).toContain("Family");
    expect(results.text()).not.toContain("Personal");
    expect(wrapper.find('[data-testid="settings-spaces"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="settings-devices"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="settings-backup"]').exists()).toBe(false);

    await search.setValue("");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-search-results"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="spaces-settings-section-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="devices-settings-section-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="backup-settings-section-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="theme-selector-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="about-card-stub"]').exists()).toBe(true);
  });

  it("renders each matching action in its section group", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("backup");
    await flushUi();

    const results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Backup");
    expect(results.text()).toContain("Export backup");
    expect(results.text()).toContain("Import backup");
    expect(wrapper.findAll('[data-testid="settings-search-group"]')).toHaveLength(1);
  });

  it("shows only matching result groups without rendering the overview", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("backup");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-search-results"]').exists()).toBe(true);
    expect(wrapper.findAll('[data-testid="settings-search-group"]')).toHaveLength(1);
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

    const results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Devices");
    expect(results.text()).toContain("Kitchen iPad");
    expect(results.text()).toContain("Online");
    expect(results.text()).not.toContain("Hall Phone");
    expect(wrapper.find('[data-testid="add-device-link"]').exists()).toBe(false);

    await search.setValue("peer-hall-uuid");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-search-empty"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("No settings found");
    expect(wrapper.text()).not.toContain("Hall Phone");
  });

  it("links paired device search results to device details", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("kitchen");
    await flushUi();

    const results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Kitchen iPad");
    expect(results.find('[data-to="/settings/device/peer-kitchen-uuid"]').exists()).toBe(true);
  });

  it("matches visible section headings in settings search", async () => {
    const wrapper = mountSettingsView();
    await flushUi();

    const search = wrapper.find('[data-testid="settings-search-input"]');
    await search.setValue("appearance");
    await flushUi();

    let results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Appearance");
    expect(results.text()).toContain("Theme");
    expect(wrapper.findAll('[data-testid="settings-search-group"]')).toHaveLength(1);

    await search.setValue("about");
    await flushUi();

    results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("About");
    expect(results.text()).toContain("Version");
    expect(wrapper.find('[data-testid="settings-search-empty"]').exists()).toBe(false);
  });

  it("includes the static Add device row in device search results", async () => {
    mockSettingsStores();
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("add device");
    await flushUi();

    const results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Devices");
    expect(results.text()).toContain("Add device");
    expect(results.find('[data-to="/settings/add-device"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="settings-search-empty"]').exists()).toBe(false);
  });

  it("includes automatic updates in search only when startup updates are supported", async () => {
    (invoke as jest.Mock).mockResolvedValueOnce(true);
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("automatic updates");
    await flushUi();

    const results = wrapper.find('[data-testid="settings-search-results"]');
    expect(results.exists()).toBe(true);
    expect(results.text()).toContain("Updates");
    expect(results.text()).toContain("Automatic updates");
    expect(wrapper.find('[data-testid="settings-search-empty"]').exists()).toBe(false);
  });

  it("omits automatic updates from search when startup updates are unsupported", async () => {
    (invoke as jest.Mock).mockResolvedValueOnce(false);
    const wrapper = mountSettingsView();
    await flushUi();

    await wrapper.find('[data-testid="settings-search-input"]').setValue("automatic updates");
    await flushUi();

    expect(wrapper.find('[data-testid="settings-search-empty"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("No settings found");
    expect(wrapper.text()).not.toContain("Automatic updates");
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
