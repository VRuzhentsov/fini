import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import DeviceView from "../../views/DeviceView.vue";
import { useDeviceStore } from "../../stores/device";
import { useSpaceStore } from "../../stores/space";

jest.mock("vue-router", () => ({
  useRoute: () => ({ params: { id: "peer-device-123" } }),
  useRouter: () => ({ push: jest.fn() }),
}));

jest.mock("../../stores/device", () => ({
  useDeviceStore: jest.fn(),
}));

jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
}));

async function flushUi() {
  for (let i = 0; i < 4; i += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

describe("DeviceView mapped spaces sync labels", () => {
  beforeEach(() => {
    const deviceStoreMock = {
      findPairedDevice: jest.fn().mockReturnValue({
        peer_device_id: "peer-device-123",
        display_name: "peer-host",
        paired_at: "2026-04-07T11:00:00.000Z",
        last_seen_at: "2026-04-07T11:05:00.000Z",
        pair_state: "paired",
      }),
      isDeviceOnline: jest.fn().mockReturnValue(true),
      getSpaceSyncStatus: jest.fn().mockReturnValue({
        peer_device_id: "peer-device-123",
        pending_event_count: 0,
        outbox_event_count: 10,
        acked_event_count: 10,
        mapped_space_ids: ["1", "2", "foo-space-1"],
        seen_event_count: 10,
        tombstone_count: 0,
      }),
      getLastSyncedAt: jest.fn().mockReturnValue("2026-04-07T12:34:56.000Z"),
      getLastSyncedAtBySpace: jest.fn().mockReturnValue({
        "1": "2026-04-07T12:34:56.000Z",
        "2": "2026-04-07T12:35:56.000Z",
        "foo-space-1": "2026-04-07T12:36:56.000Z",
      }),
      getMappedSpaceIds: jest.fn().mockReturnValue(["1", "2", "foo-space-1"]),
      getUnresolvedCustomSpaces: jest.fn().mockReturnValue([]),
      shortDeviceId: jest.fn().mockReturnValue("ce-123"),
      hydrate: jest.fn().mockResolvedValue(undefined),
      runSpaceSyncTick: jest.fn().mockResolvedValue(undefined),
      loadMappedSpaces: jest.fn().mockResolvedValue(["1", "2", "foo-space-1"]),
      refreshSpaceSyncStatus: jest.fn().mockResolvedValue(undefined),
      saveMappedSpaces: jest.fn().mockResolvedValue(["1", "2", "foo-space-1"]),
      resolveCustomSpaceMapping: jest.fn().mockResolvedValue(undefined),
      unpairDevice: jest.fn().mockResolvedValue(undefined),
    };

    const spaceStoreMock = {
      spaces: [
        { id: "1", name: "Personal" },
        { id: "2", name: "Family" },
        { id: "foo-space-1", name: "Foo" },
      ],
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
    };

    (useDeviceStore as unknown as jest.Mock).mockReturnValue(deviceStoreMock);
    (useSpaceStore as unknown as jest.Mock).mockReturnValue(spaceStoreMock);
  });

  it("shows last synced text for mapped Personal and Family rows", async () => {
    const wrapper = mount(DeviceView, {
      global: {
        stubs: {
          "router-link": { template: "<a><slot /></a>" },
        },
      },
    });

    await flushUi();

    const rows = wrapper.findAll("li");
    const personalRow = rows.find((row) => row.text().includes("Personal"));
    const familyRow = rows.find((row) => row.text().includes("Family"));
    const fooRow = rows.find((row) => row.text().includes("Foo"));

    expect(personalRow).toBeTruthy();
    expect(familyRow).toBeTruthy();
    expect(fooRow).toBeTruthy();

    expect(personalRow!.text()).toContain("last synced:");
    expect(familyRow!.text()).toContain("last synced:");
    expect(fooRow!.text()).toContain("last synced:");

    const syncLabelCount = (wrapper.text().match(/last synced:/g) ?? []).length;
    expect(syncLabelCount).toBeGreaterThanOrEqual(3);
  });

  it("hides IDs for embedded spaces and keeps IDs for custom spaces", async () => {
    const wrapper = mount(DeviceView, {
      global: {
        stubs: {
          "router-link": { template: "<a><slot /></a>" },
        },
      },
    });

    await flushUi();

    expect(wrapper.find('span[title="1"]').exists()).toBe(false);
    expect(wrapper.find('span[title="2"]').exists()).toBe(false);
    expect(wrapper.find('span[title="foo-space-1"]').exists()).toBe(true);
  });
});
