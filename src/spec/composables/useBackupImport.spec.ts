import { defineComponent } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import { useBackupImport } from "../../composables/useBackupImport";
import { open } from "@tauri-apps/plugin-dialog";
import { useBackupStore } from "../../stores/backup";
import { useSpaceStore } from "../../stores/space";
import { useQuestStore } from "../../stores/quest";

const mockShow = jest.fn();
const mockError = jest.fn();

jest.mock("@tauri-apps/plugin-dialog", () => ({ open: jest.fn() }));
jest.mock("../../stores/backup", () => ({ useBackupStore: jest.fn() }));
jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
  BUILTIN_SPACE_IDS: ["1", "2", "3"],
}));
jest.mock("../../stores/quest", () => ({ useQuestStore: jest.fn() }));
jest.mock("../../composables/useToast", () => ({
  useToast: () => ({ show: mockShow, error: mockError, info: jest.fn() }),
}));

function makeWrapper() {
  return mount(
    defineComponent({
      setup() { return useBackupImport(); },
      template: "<div></div>",
    }),
  );
}

const baseSpaceStore = {
  spaces: [{ id: "custom-1", name: "Work", item_order: 0, created_at: "" }],
  fetchSpaces: jest.fn().mockResolvedValue(undefined),
};
const baseQuestStore = {
  quests: [],
  fetchQuests: jest.fn().mockResolvedValue(undefined),
  fetchActiveQuest: jest.fn().mockResolvedValue(undefined),
};
const FILE_PATH = "/var/tmp/test-backup.zip";

describe("useBackupImport", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (useSpaceStore as unknown as jest.Mock).mockReturnValue(baseSpaceStore);
    (useQuestStore as unknown as jest.Mock).mockReturnValue(baseQuestStore);
  });

  it("does nothing when file picker is cancelled", async () => {
    const preflightMock = jest.fn();
    (useBackupStore as unknown as jest.Mock).mockReturnValue({ preflightImport: preflightMock, applyImport: jest.fn() });
    (open as unknown as jest.Mock).mockResolvedValue(null);

    const wrapper = makeWrapper();
    await flushPromises();

    await wrapper.vm.startImport();
    await flushPromises();

    expect(preflightMock).not.toHaveBeenCalled();
    expect(wrapper.vm.activeMapping).toBeNull();
    expect(wrapper.vm.showConflicts).toBe(false);
  });

  it("auto-applies and shows toast when preflight has no mappings and no conflicts", async () => {
    const applyMock = jest.fn().mockResolvedValue(undefined);
    const preflightMock = jest.fn().mockResolvedValue({ required_space_mappings: [], conflicts: [] });
    (useBackupStore as unknown as jest.Mock).mockReturnValue({ preflightImport: preflightMock, applyImport: applyMock });
    (open as unknown as jest.Mock).mockResolvedValue(FILE_PATH);

    const wrapper = makeWrapper();
    await flushPromises();

    await wrapper.vm.startImport();
    await flushPromises();

    expect(preflightMock).toHaveBeenCalledWith(FILE_PATH, []);
    expect(applyMock).toHaveBeenCalledWith(FILE_PATH, [], []);
    expect(mockShow).toHaveBeenCalledWith("Backup imported", "success", 3000);
    // state is cleaned up after apply
    expect(wrapper.vm.activeMapping).toBeNull();
    expect(wrapper.vm.showConflicts).toBe(false);
  });

  it("sets activeMapping when preflight requires space mapping", async () => {
    const pending = [{ backup_space_id: "abc", backup_space_name: "Work" }];
    const preflightMock = jest.fn().mockResolvedValue({ required_space_mappings: pending, conflicts: [] });
    (useBackupStore as unknown as jest.Mock).mockReturnValue({ preflightImport: preflightMock, applyImport: jest.fn() });
    (open as unknown as jest.Mock).mockResolvedValue(FILE_PATH);

    const wrapper = makeWrapper();
    await flushPromises();

    await wrapper.vm.startImport();
    await flushPromises();

    expect(wrapper.vm.activeMapping).toEqual({ backup_space_id: "abc", backup_space_name: "Work" });
    expect(wrapper.vm.showConflicts).toBe(false);
    expect(wrapper.vm.totalMappings).toBe(1);
  });

  it("shows conflicts when preflight has no pending mappings but has conflicts", async () => {
    const conflicts = [
      { entity_type: "quest", id: "q-1", title: "Q", local_summary: "L", backup_summary: "B", local: {}, backup: {} },
    ];
    const preflightMock = jest.fn().mockResolvedValue({ required_space_mappings: [], conflicts });
    (useBackupStore as unknown as jest.Mock).mockReturnValue({ preflightImport: preflightMock, applyImport: jest.fn() });
    (open as unknown as jest.Mock).mockResolvedValue(FILE_PATH);

    const wrapper = makeWrapper();
    await flushPromises();

    await wrapper.vm.startImport();
    await flushPromises();

    expect(wrapper.vm.showConflicts).toBe(true);
    expect(wrapper.vm.conflicts).toHaveLength(1);
    expect(wrapper.vm.activeMapping).toBeNull();
  });

  it("accumulates mapping and re-runs preflight after confirmMapping", async () => {
    // First preflight: 1 mapping required. Second: none, no conflicts → auto-apply.
    const applyMock = jest.fn().mockResolvedValue(undefined);
    const preflightMock = jest.fn()
      .mockResolvedValueOnce({ required_space_mappings: [{ backup_space_id: "abc", backup_space_name: "Work" }], conflicts: [] })
      .mockResolvedValueOnce({ required_space_mappings: [], conflicts: [] });
    (useBackupStore as unknown as jest.Mock).mockReturnValue({ preflightImport: preflightMock, applyImport: applyMock });
    (open as unknown as jest.Mock).mockResolvedValue(FILE_PATH);

    const wrapper = makeWrapper();
    await flushPromises();

    await wrapper.vm.startImport();
    await flushPromises();
    expect(wrapper.vm.activeMapping).toEqual({ backup_space_id: "abc", backup_space_name: "Work" });

    await wrapper.vm.confirmMapping({ mode: "create_new" });
    await flushPromises();

    // Second preflight had no conflicts → auto-apply ran
    expect(applyMock).toHaveBeenCalledWith(
      FILE_PATH,
      [{ backup_space_id: "abc", mode: "create_new", local_space_id: undefined }],
      [],
    );
    expect(mockShow).toHaveBeenCalledWith("Backup imported", "success", 3000);
  });
});
