import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import DeviceView from "../../views/settings/DeviceView.vue";
import { useDeviceStore } from "../../stores/device";
import { useSpaceStore } from "../../stores/space";
import { ChannelColor, ChannelKind, ChannelProblem, ChannelState } from "../../utils/channel";

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

// One row per channel, as the backend sends it (ADR-0008 D19): both on and
// green is a pair whose peer was heard on both channels just now.
function channelRow(kind: ChannelKind, overrides: Record<string, unknown> = {}) {
  return { kind, state: ChannelState.On, color: ChannelColor.Green, problem: null, ...overrides };
}

const GREEN_ROWS = [channelRow(ChannelKind.Network), channelRow(ChannelKind.Bluetooth)];

function pairedDevice(overrides: Record<string, unknown> = {}) {
  return {
    peer_device_id: "peer-device-123",
    display_name: "peer-host",
    paired_at: "2026-04-07T11:00:00.000Z",
    last_seen_at: "2026-04-07T11:05:00.000Z",
    pair_state: "paired",
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
    getChannelStatuses: jest.fn().mockReturnValue(GREEN_ROWS),
    getSyncQueue: jest.fn().mockReturnValue(null),
    refreshSyncQueue: jest.fn().mockResolvedValue(null),
    shortDeviceId: jest.fn().mockReturnValue("ce-123"),
    hydrate: jest.fn().mockResolvedValue(undefined),
    runSpaceSyncTick: jest.fn().mockResolvedValue(undefined),
    loadMappedSpaces: jest.fn().mockResolvedValue([]),
    refreshSpaceSyncStatus: jest.fn().mockResolvedValue(undefined),
    refreshChannelStatuses: jest.fn().mockResolvedValue(undefined),
    setChannelEnabled: jest.fn().mockResolvedValue([]),
    unlinkChannel: jest.fn().mockResolvedValue([]),
    watchPresence: jest.fn().mockResolvedValue(undefined),
    beginChannelSetup: jest.fn().mockResolvedValue(undefined),
    channelSetupStatus: jest.fn().mockResolvedValue(null),
    endChannelSetup: jest.fn().mockResolvedValue([]),
    enterAddMode: jest.fn().mockResolvedValue(undefined),
    leaveAddMode: jest.fn().mockResolvedValue(undefined),
    cancelOutgoingRequest: jest.fn(),
    outgoingRequest: null,
    incomingRequests: [],
    discoveredByChannel: { [ChannelKind.Network]: [], [ChannelKind.Bluetooth]: [] },
    pairCompletedAt: null,
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

  it("draws one row per channel in the colour the backend chose", async () => {
    deviceStoreMock.getChannelStatuses.mockReturnValue([
      channelRow(ChannelKind.Network),
      channelRow(ChannelKind.Bluetooth, { color: ChannelColor.Grey }),
    ]);
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="channel-status-row"]');
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("Network");
    expect(rows[0].attributes("data-channel-color")).toBe(ChannelColor.Green);
    expect(rows[1].text()).toContain("Bluetooth");
    expect(rows[1].attributes("data-channel-color")).toBe(ChannelColor.Grey);
  });

  it("searches for the peer only while the page is open", async () => {
    const wrapper = mountView();
    await flushUi();
    expect(deviceStoreMock.watchPresence).toHaveBeenCalledWith(true);

    wrapper.unmount();
    expect(deviceStoreMock.watchPresence).toHaveBeenCalledWith(false);
  });

  it("turns a channel off through its own switch", async () => {
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="channel-status-row"]');
    await rows[0].find('[data-testid="channel-switch"]').trigger("change");
    await flushUi();

    expect(deviceStoreMock.setChannelEnabled).toHaveBeenCalledWith(
      "peer-device-123",
      ChannelKind.Network,
      false,
    );
  });

  it("turns an off channel back on through the same switch", async () => {
    deviceStoreMock.getChannelStatuses.mockReturnValue([
      channelRow(ChannelKind.Network),
      channelRow(ChannelKind.Bluetooth, { state: ChannelState.Off, color: ChannelColor.Off }),
    ]);
    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="channel-status-row"]');
    await rows[1].find('[data-testid="channel-switch"]').trigger("change");
    await flushUi();

    expect(deviceStoreMock.setChannelEnabled).toHaveBeenCalledWith(
      "peer-device-123",
      ChannelKind.Bluetooth,
      true,
    );
  });

  // Unlinking is a second, deliberate act (ADR-0008 D14): the control
  // exists only on a channel that is already off.
  it("offers unlink only once the channel is off", async () => {
    deviceStoreMock.getChannelStatuses.mockReturnValue([
      channelRow(ChannelKind.Network),
      channelRow(ChannelKind.Bluetooth, { state: ChannelState.Off, color: ChannelColor.Off }),
    ]);

    const wrapper = mountView();
    await flushUi();

    const rows = wrapper.findAll('[data-testid="channel-status-row"]');
    expect(rows[0].find('[data-testid="unlink-channel"]').exists()).toBe(false);

    await rows[1].find('[data-testid="unlink-channel"]').trigger("click");
    await flushUi();

    expect(deviceStoreMock.unlinkChannel).toHaveBeenCalledWith("peer-device-123", ChannelKind.Bluetooth);
  });

  // A channel that does not exist has nothing to switch or forget -- only
  // Add, which opens the setup dialog for that channel (ADR-0008 D20).
  it("opens the setup dialog from Add on a channel that does not exist", async () => {
    deviceStoreMock.getChannelStatuses.mockReturnValue([
      channelRow(ChannelKind.Network),
      channelRow(ChannelKind.Bluetooth, { state: ChannelState.None, color: ChannelColor.None }),
    ]);

    const wrapper = mountView();
    await flushUi();

    const row = wrapper.findAll('[data-testid="channel-status-row"]')[1];
    expect(row.find('[data-testid="channel-switch"]').exists()).toBe(false);
    expect(row.find('[data-testid="unlink-channel"]').exists()).toBe(false);
    expect(document.body.querySelector('[data-testid="pair-device-dialog"]')).toBeNull();

    await row.find('[data-testid="add-channel"]').trigger("click");
    await flushUi();

    expect(deviceStoreMock.beginChannelSetup).toHaveBeenCalledWith("peer-device-123", ChannelKind.Bluetooth);
    wrapper.unmount();
  });

  // ⓘ appears only on orange, and names this device's problem -- a peer
  // that is simply away is grey, with nothing to explain (ADR-0008 D19).
  it("explains a problem on this device behind ⓘ, and only then", async () => {
    deviceStoreMock.getChannelStatuses.mockReturnValue([
      channelRow(ChannelKind.Network, { color: ChannelColor.Grey }),
      channelRow(ChannelKind.Bluetooth, { color: ChannelColor.Orange, problem: ChannelProblem.BluetoothUnavailable }),
    ]);

    const wrapper = mountView();
    await flushUi();

    const [networkRow, bluetoothRow] = wrapper.findAll('[data-testid="channel-status-row"]');
    expect(networkRow.find('[data-testid="channel-problem-info"]').exists()).toBe(false);
    expect(bluetoothRow.find('[data-testid="channel-problem-info"]').exists()).toBe(true);
    expect(bluetoothRow.find('[data-testid="channel-problem-popup"]').text()).toContain(
      "on this device",
    );
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
