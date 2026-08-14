import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import DeviceView from "../../views/DeviceView.vue";
import { useDeviceStore } from "../../stores/device";
import { useSpaceStore } from "../../stores/space";

const mockRouterPush = jest.fn();
jest.mock("vue-router", () => ({
  useRoute: () => ({ params: { id: "peer-device-123" } }),
  useRouter: () => ({ push: mockRouterPush }),
}));

jest.mock("../../stores/device", () => ({
  useDeviceStore: jest.fn(),
}));

jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
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
        bluetooth_enabled: true,
        bluetooth_address: "AA:BB:CC:DD:EE:FF",
        bluetooth_last_verified_at: "2026-04-07T11:01:00.000Z",
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
      getTransportStatuses: jest.fn().mockReturnValue([
        {
          kind: "network",
          preferred: true,
          state: { state: "live" },
        },
        {
          kind: "bluetooth",
          preferred: false,
          state: { state: "configured", reliable: true },
        },
      ]),
      shortDeviceId: jest.fn().mockReturnValue("ce-123"),
      hydrate: jest.fn().mockResolvedValue(undefined),
      runSpaceSyncTick: jest.fn().mockResolvedValue(undefined),
      loadMappedSpaces: jest.fn().mockResolvedValue(["1", "2", "foo-space-1"]),
      refreshSpaceSyncStatus: jest.fn().mockResolvedValue(undefined),
      refreshTransportStatuses: jest.fn().mockResolvedValue(undefined),
      setBluetoothTransport: jest.fn().mockResolvedValue(undefined),
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

  it("shows last synced date and time for mapped rows", async () => {
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
    expect(personalRow!.text()).toContain("2026");
    expect(familyRow!.text()).toContain("last synced:");
    expect(familyRow!.text()).toContain("2026");
    expect(fooRow!.text()).toContain("last synced:");
    expect(fooRow!.text()).toContain("2026");

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

  it("shows separate network and Bluetooth transport rows", async () => {
    const wrapper = mount(DeviceView, {
      global: {
        stubs: {
          "router-link": { template: "<a><slot /></a>" },
        },
      },
    });

    await flushUi();

    const rows = wrapper.findAll('[data-testid="transport-status-row"]');
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("Network");
    expect(rows[0].text()).toContain("preferred");
    expect(rows[1].text()).toContain("Bluetooth");
    expect(rows[1].text()).toContain("Ready");
  });
});

describe("DeviceView Bluetooth manual entry, search, and unpair", () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let deviceStoreMock: any;

  beforeEach(() => {
    // jsdom doesn't implement <dialog>'s showModal/close -- stub them so
    // openUnpairDialog/confirmUnpair don't throw when this component calls
    // through the template ref.
    HTMLDialogElement.prototype.showModal ??= jest.fn();
    HTMLDialogElement.prototype.close ??= jest.fn();
    mockRouterPush.mockClear();

    deviceStoreMock = {
      findPairedDevice: jest.fn().mockReturnValue({
        peer_device_id: "peer-device-123",
        display_name: "peer-host",
        paired_at: "2026-04-07T11:00:00.000Z",
        last_seen_at: "2026-04-07T11:05:00.000Z",
        pair_state: "paired",
        bluetooth_enabled: false,
        bluetooth_address: null,
        bluetooth_last_verified_at: null,
      }),
      isDeviceOnline: jest.fn().mockReturnValue(true),
      getSpaceSyncStatus: jest.fn().mockReturnValue({
        peer_device_id: "peer-device-123",
        pending_event_count: 0,
        outbox_event_count: 0,
        acked_event_count: 0,
        mapped_space_ids: [],
        seen_event_count: 0,
        tombstone_count: 0,
      }),
      getLastSyncedAt: jest.fn().mockReturnValue(null),
      getLastSyncedAtBySpace: jest.fn().mockReturnValue({}),
      getMappedSpaceIds: jest.fn().mockReturnValue([]),
      getUnresolvedCustomSpaces: jest.fn().mockReturnValue([]),
      getTransportStatuses: jest.fn().mockReturnValue([
        {
          kind: "network",
          enabled: true,
          available: true,
          preferred: true,
          connected: true,
          detail: "Available",
        },
        {
          kind: "bluetooth",
          enabled: false,
          available: false,
          preferred: false,
          connected: false,
          detail: "Disabled for this Fini pair",
        },
      ]),
      shortDeviceId: jest.fn().mockReturnValue("ce-123"),
      hydrate: jest.fn().mockResolvedValue(undefined),
      runSpaceSyncTick: jest.fn().mockResolvedValue(undefined),
      loadMappedSpaces: jest.fn().mockResolvedValue([]),
      refreshSpaceSyncStatus: jest.fn().mockResolvedValue(undefined),
      refreshTransportStatuses: jest.fn().mockResolvedValue(undefined),
      refreshLiveConnectedState: jest.fn().mockResolvedValue(undefined),
      setBluetoothTransport: jest.fn().mockResolvedValue(undefined),
      findBluetoothAddress: jest.fn(),
      saveMappedSpaces: jest.fn().mockResolvedValue([]),
      resolveCustomSpaceMapping: jest.fn().mockResolvedValue(undefined),
      unpairDevice: jest.fn().mockResolvedValue(undefined),
    };

    const spaceStoreMock = {
      spaces: [],
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
    };

    (useDeviceStore as unknown as jest.Mock).mockReturnValue(deviceStoreMock);
    (useSpaceStore as unknown as jest.Mock).mockReturnValue(spaceStoreMock);
  });

  function mountView() {
    return mount(DeviceView, {
      global: {
        stubs: {
          "router-link": { template: "<a><slot /></a>" },
        },
      },
    });
  }

  it("masks manually typed input with colons and sends the normalized address on Enable", async () => {
    const wrapper = mountView();
    await flushUi();

    const input = wrapper.find('[data-testid="bluetooth-address-input"]');
    await input.setValue("aabbccddeeff");
    await flushUi();

    expect((input.element as HTMLInputElement).value).toBe("AA:BB:CC:DD:EE:FF");

    await wrapper.find('[data-testid="enable-bluetooth-transport"]').trigger("click");
    await flushUi();

    expect(deviceStoreMock.setBluetoothTransport).toHaveBeenCalledWith(
      "peer-device-123",
      true,
      "AA:BB:CC:DD:EE:FF",
    );
  });

  it("masks a paste with existing separators the same way as raw digits", async () => {
    const wrapper = mountView();
    await flushUi();

    const input = wrapper.find('[data-testid="bluetooth-address-input"]');
    await input.setValue("aa-bb-cc-dd-ee-ff");
    await flushUi();

    expect((input.element as HTMLInputElement).value).toBe("AA:BB:CC:DD:EE:FF");
  });

  it("finds an address via Bluetooth scan and displays it masked", async () => {
    deviceStoreMock.findBluetoothAddress.mockResolvedValue("AA:BB:CC:DD:EE:FF");
    const wrapper = mountView();
    await flushUi();

    await wrapper.find('[data-testid="find-bluetooth-address"]').trigger("click");
    await flushUi();

    expect(deviceStoreMock.findBluetoothAddress).toHaveBeenCalledWith("peer-device-123");
    const input = wrapper.find('[data-testid="bluetooth-address-input"]');
    expect((input.element as HTMLInputElement).value).toBe("AA:BB:CC:DD:EE:FF");
    expect(wrapper.text()).toContain("Found and confirmed nearby.");
  });

  it("shows a not-found message when the Bluetooth scan finds nothing", async () => {
    deviceStoreMock.findBluetoothAddress.mockResolvedValue(null);
    const wrapper = mountView();
    await flushUi();

    await wrapper.find('[data-testid="find-bluetooth-address"]').trigger("click");
    await flushUi();

    expect(wrapper.text()).toContain("Not found within 60s");
  });

  it("surfaces a scan error instead of a silent not-found", async () => {
    deviceStoreMock.findBluetoothAddress.mockRejectedValue(new Error("adapter unavailable"));
    const wrapper = mountView();
    await flushUi();

    await wrapper.find('[data-testid="find-bluetooth-address"]').trigger("click");
    await flushUi();

    expect(wrapper.text()).toContain("adapter unavailable");
    expect(wrapper.text()).not.toContain("Not found within 60s");
  });

  it("unpairs the device and navigates back to settings on confirm", async () => {
    const wrapper = mountView();
    await flushUi();

    const dialog = wrapper.find("dialog");
    const confirmButton = dialog.findAll("button").find((btn) => btn.text() === "Unpair");
    expect(confirmButton).toBeTruthy();

    await confirmButton!.trigger("click");
    await flushUi();

    expect(deviceStoreMock.unpairDevice).toHaveBeenCalledWith("peer-device-123");
    expect(mockRouterPush).toHaveBeenCalledWith("/settings");
  });
});
