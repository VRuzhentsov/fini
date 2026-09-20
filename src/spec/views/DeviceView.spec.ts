import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import DeviceView from "../../views/settings/DeviceView.vue";
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

// `spaceCss` is re-exported here as well as `isBuiltinSpace`: SyncQueueSection
// imports it for the space-colour dot, and a module mock that omits a name the
// component under test imports yields `undefined` at render time rather than a
// missing-export error.
jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  isBuiltinSpace: (id: string) => ["1", "2", "3"].includes(id),
  spaceCss: (id: string) =>
    ({ "1": "space-color-personal", "2": "space-color-family", "3": "space-color-work" })[id] ?? "",
}));

async function flushUi() {
  for (let i = 0; i < 4; i += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

const GREEN_ROWS = [
  { kind: "network", primary: true, state: { state: "configured", code: null } },
  { kind: "bluetooth", primary: false, state: { state: "configured", code: null } },
];

function pairedDevice(overrides: Record<string, unknown> = {}) {
  return {
    peer_device_id: "peer-device-123",
    display_name: "peer-host",
    paired_at: "2026-04-07T11:00:00.000Z",
    last_seen_at: "2026-04-07T11:05:00.000Z",
    pair_state: "paired",
    bluetooth_enabled: true,
    bluetooth_address: "AA:BB:CC:DD:EE:FF",
    bluetooth_last_verified_at: "2026-04-07T11:01:00.000Z",
    preferred_transport: null,
    preferred_transport_set_at: null,
    network_enabled: true,
    ...overrides,
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function storeMock(overrides: Record<string, unknown> = {}): any {
  return {
    findPairedDevice: jest.fn().mockReturnValue(pairedDevice()),
    isDeviceOnline: jest.fn().mockReturnValue(true),
    getSpaceSyncStatus: jest.fn().mockReturnValue(null),
    getLastSyncedAt: jest.fn().mockReturnValue(null),
    getLastSyncedAtBySpace: jest.fn().mockReturnValue({}),
    getMappedSpaceIds: jest.fn().mockReturnValue([]),
    getUnresolvedCustomSpaces: jest.fn().mockReturnValue([]),
    getTransportStatuses: jest.fn().mockReturnValue(GREEN_ROWS),
    getSyncQueue: jest.fn().mockReturnValue(null),
    refreshSyncQueue: jest.fn().mockResolvedValue(null),
    shortDeviceId: jest.fn().mockReturnValue("ce-123"),
    hydrate: jest.fn().mockResolvedValue(undefined),
    runSpaceSyncTick: jest.fn().mockResolvedValue(undefined),
    loadMappedSpaces: jest.fn().mockResolvedValue([]),
    refreshSpaceSyncStatus: jest.fn().mockResolvedValue(undefined),
    refreshTransportStatuses: jest.fn().mockResolvedValue(undefined),
    refreshLiveConnectedState: jest.fn().mockResolvedValue(undefined),
    setBluetoothTransport: jest.fn().mockResolvedValue(undefined),
    setNetworkTransport: jest.fn().mockResolvedValue(undefined),
    probeBluetoothAdapter: jest.fn().mockResolvedValue(true),
    setPreferredTransport: jest.fn().mockResolvedValue(undefined),
    retryBluetoothDial: jest.fn().mockResolvedValue(undefined),
    findBluetoothAddress: jest.fn().mockResolvedValue(null),
    saveMappedSpaces: jest.fn().mockResolvedValue([]),
    resolveCustomSpaceMapping: jest.fn().mockResolvedValue(undefined),
    unpairDevice: jest.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function mountView() {
  return mount(DeviceView, {
    global: { stubs: { "router-link": { template: "<a><slot /></a>" } } },
  });
}

describe("DeviceView shared spaces", () => {
  beforeEach(() => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        getMappedSpaceIds: jest.fn().mockReturnValue(["1", "2", "foo-space-1"]),
        loadMappedSpaces: jest.fn().mockResolvedValue(["1", "2", "foo-space-1"]),
        getLastSyncedAtBySpace: jest.fn().mockReturnValue({
          "1": "2026-04-07T12:34:56.000Z",
          "2": "2026-04-07T12:35:56.000Z",
          "foo-space-1": "2026-04-07T12:36:56.000Z",
        }),
      }),
    );
    (useSpaceStore as unknown as jest.Mock).mockReturnValue({
      spaces: [
        { id: "1", name: "Personal" },
        { id: "2", name: "Family" },
        { id: "foo-space-1", name: "Foo" },
      ],
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
    });
  });

  it("shows last synced date and time for mapped rows", async () => {
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="mapped-space-row"]');
    const personalRow = rows.find((row) => row.text().includes("Personal"));
    const fooRow = rows.find((row) => row.text().includes("Foo"));

    expect(personalRow!.text()).toContain("last synced:");
    expect(personalRow!.text()).toContain("2026");
    expect(fooRow!.text()).toContain("last synced:");
  });

  it("hides IDs for embedded spaces and keeps IDs for custom spaces", async () => {
    const wrapper = mountView();
    await flushUi();

    expect(wrapper.find('span[title="1"]').exists()).toBe(false);
    expect(wrapper.find('span[title="2"]').exists()).toBe(false);
    expect(wrapper.find('span[title="foo-space-1"]').exists()).toBe(true);
  });
});

describe("DeviceView channels", () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let deviceStoreMock: any;

  beforeEach(() => {
    HTMLDialogElement.prototype.showModal ??= jest.fn();
    HTMLDialogElement.prototype.close ??= jest.fn();
    mockRouterPush.mockClear();

    deviceStoreMock = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(deviceStoreMock);
    (useSpaceStore as unknown as jest.Mock).mockReturnValue({
      spaces: [],
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
    });
  });

  it("shows one row per channel, named and labelled in plain language", async () => {
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="transport-status-row"]');
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("Network");
    expect(rows[0].text()).toContain("Connected");
    expect(rows[1].text()).toContain("Bluetooth");
    expect(rows[1].text()).toContain("Connected");
  });

  it("pins a channel when its row is clicked", async () => {
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="transport-status-row"]');
    await rows[1].find("button").trigger("click");
    await flushUi();

    expect(deviceStoreMock.setPreferredTransport).toHaveBeenCalledWith("peer-device-123", "bluetooth");
  });

  it("stars the manually pinned channel, not the automatic choice", async () => {
    // The fixture marks "network" as primary, but this pair is pinned to
    // Bluetooth -- the star follows the user's pin.
    deviceStoreMock.findPairedDevice.mockReturnValue(
      pairedDevice({ preferred_transport: "bluetooth" }),
    );

    const wrapper = mountView();
    await flushUi();

    const stars = wrapper.findAll('[data-testid="channel-star"]');
    expect(stars).toHaveLength(2);
    expect(stars[0].attributes("data-starred")).toBe("false");
    expect(stars[1].attributes("data-starred")).toBe("true");
  });

  it("turns a channel off through its own switch", async () => {
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="transport-status-row"]');
    await rows[0].find('[data-testid="channel-switch"]').trigger("click");
    await flushUi();

    expect(deviceStoreMock.setNetworkTransport).toHaveBeenCalledWith("peer-device-123", false);
  });

  // Switching a channel on asks the radio directly rather than waiting for
  // the background dial loop, so the row can say "on, waiting" immediately
  // instead of up to a tick later.
  it("probes the adapter when Bluetooth is switched on", async () => {
    deviceStoreMock.findPairedDevice.mockReturnValue(pairedDevice({ bluetooth_enabled: false }));
    deviceStoreMock.getTransportStatuses.mockReturnValue([
      GREEN_ROWS[0],
      {
        kind: "bluetooth",
        primary: false,
        state: { state: "unconfigured", code: { code: "bluetooth_disabled" } },
      },
    ]);

    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="transport-status-row"]');
    await rows[1].find('[data-testid="channel-switch"]').trigger("click");
    await flushUi();

    expect(deviceStoreMock.setBluetoothTransport).toHaveBeenCalledWith("peer-device-123", true);
    expect(deviceStoreMock.probeBluetoothAdapter).toHaveBeenCalled();
  });

  it("explains a dead local radio without blaming the peer", async () => {
    deviceStoreMock.getTransportStatuses.mockReturnValue([
      GREEN_ROWS[0],
      {
        kind: "bluetooth",
        primary: false,
        state: { state: "unconfigured", code: { code: "bluetooth_adapter_off" } },
      },
    ]);

    const wrapper = mountView();
    await flushUi();

    const bluetoothRow = wrapper.findAll('[data-testid="transport-status-row"]')[1];
    expect(bluetoothRow.attributes("data-channel-state")).toBe("waiting");
    expect(bluetoothRow.text()).toContain("On, waiting");
    // Shown without being asked for, and it names this machine rather than
    // the peer.
    expect(bluetoothRow.text()).toContain("Bluetooth is off on this computer");
    expect(bluetoothRow.text()).not.toContain("isn't nearby");
  });

  it("names the peer when the peer is the one out of reach", async () => {
    deviceStoreMock.getTransportStatuses.mockReturnValue([
      GREEN_ROWS[0],
      {
        kind: "bluetooth",
        primary: false,
        state: { state: "unconfigured", code: { code: "bluetooth_peer_not_nearby" } },
      },
    ]);

    const wrapper = mountView();
    await flushUi();

    const bluetoothRow = wrapper.findAll('[data-testid="transport-status-row"]')[1];
    expect(bluetoothRow.attributes("data-channel-state")).toBe("down");
    await bluetoothRow.find('[data-testid="transport-status-info"]').trigger("click");
    await flushUi();
    expect(bluetoothRow.text()).toContain("peer-host isn't nearby");
  });
});

describe("DeviceView unlink", () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let deviceStoreMock: any;

  beforeEach(() => {
    HTMLDialogElement.prototype.showModal ??= jest.fn();
    HTMLDialogElement.prototype.close ??= jest.fn();
    mockRouterPush.mockClear();

    deviceStoreMock = storeMock({
      getMappedSpaceIds: jest.fn().mockReturnValue(["1"]),
      loadMappedSpaces: jest.fn().mockResolvedValue(["1"]),
    });
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(deviceStoreMock);
    (useSpaceStore as unknown as jest.Mock).mockReturnValue({
      spaces: [{ id: "1", name: "Personal" }],
      fetchSpaces: jest.fn().mockResolvedValue(undefined),
    });
  });

  // The confirmation has to say what actually stops, by name, and that
  // nothing is lost -- a generic warning gives the user nothing to decide on.
  it("names the spaces that stop syncing and promises nothing is deleted", async () => {
    const wrapper = mountView();
    await flushUi();

    const dialogText = wrapper.find('[data-testid="unlink-dialog"]').text();
    expect(dialogText).toContain("Personal");
    expect(dialogText).toContain("Nothing is deleted");
  });

  it("unlinks the device and navigates back to settings on confirm", async () => {
    const wrapper = mountView();
    await flushUi();

    const confirmButton = wrapper
      .find('[data-testid="unlink-dialog"]')
      .findAll("button")
      .find((button) => button.text() === "Unlink");
    expect(confirmButton).toBeTruthy();

    await confirmButton!.trigger("click");
    await flushUi();

    expect(deviceStoreMock.unpairDevice).toHaveBeenCalledWith("peer-device-123");
    expect(mockRouterPush).toHaveBeenCalledWith("/settings");
  });
});
