import { mount } from "@vue/test-utils";
import DevicesSettingsSection from "../../components/settings/DevicesSettingsSection.vue";
import { useDeviceStore } from "../../stores/device";

jest.mock("../../stores/device", () => ({
  useDeviceStore: jest.fn(),
}));

jest.mock("../../components/settings/DeviceSetupDialog.vue", () => ({
  __esModule: true,
  default: { name: "DeviceSetupDialog", template: "<div />" },
}));

describe("DevicesSettingsSection", () => {
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => jest.useRealTimers());

  it("keeps presence and the device rows current only while visible", () => {
    const store = {
      incomingRequests: [],
      pairedDevices: [{ peer_device_id: "peer-1", display_name: "Pixel 8" }],
      getChannelStatuses: jest.fn().mockReturnValue([]),
      refreshChannelStatuses: jest.fn().mockResolvedValue([]),
      watchPresence: jest.fn().mockResolvedValue(undefined),
    };
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);

    const wrapper = mount(DevicesSettingsSection, { global: { stubs: { Teleport: true, RouterLink: true } } });
    expect(store.watchPresence).toHaveBeenCalledWith(true);
    expect(store.refreshChannelStatuses).toHaveBeenCalledWith("peer-1");

    jest.advanceTimersByTime(5_000);
    expect(store.refreshChannelStatuses).toHaveBeenCalledTimes(2);

    wrapper.unmount();
    expect(store.watchPresence).toHaveBeenLastCalledWith(false);
    jest.advanceTimersByTime(10_000);
    expect(store.refreshChannelStatuses).toHaveBeenCalledTimes(2);
  });
});
