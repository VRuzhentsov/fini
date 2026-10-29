import { mount } from "@vue/test-utils";
import DevicesSettingsSection from "../../components/settings/DevicesSettingsSection.vue";
import { useDeviceStore } from "../../stores/device.store";

jest.mock("../../stores/device.store", () => ({
  useDeviceStore: jest.fn(),
}));

jest.mock("../../components/settings/DeviceSetupDialog.vue", () => ({
  __esModule: true,
  default: { name: "DeviceSetupDialog", template: "<div />" },
}));

describe("DevicesSettingsSection", () => {
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => jest.useRealTimers());

  it("reads the device rows once when it opens, with no polling", () => {
    const store = {
      incomingRequests: [],
      pairedDevices: [{ peer_device_id: "peer-1", display_name: "Pixel 8" }],
      getChannelStatuses: jest.fn().mockReturnValue([]),
      refreshAllChannelStatuses: jest.fn().mockResolvedValue(undefined),
    };
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);

    const wrapper = mount(DevicesSettingsSection, { global: { stubs: { Teleport: true, RouterLink: true } } });
    expect(store.refreshAllChannelStatuses).toHaveBeenCalledTimes(1);

    jest.advanceTimersByTime(10_000);
    expect(store.refreshAllChannelStatuses).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });
});
