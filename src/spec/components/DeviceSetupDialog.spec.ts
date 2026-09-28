import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import DeviceSetupDialog from "../../components/settings/DeviceSetupDialog.vue";
import { useDeviceStore } from "../../stores/device";
import { ChannelKind } from "../../utils/channel";

jest.mock("../../stores/device", () => ({
  useDeviceStore: jest.fn(),
}));

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function storeMock(overrides: Record<string, unknown> = {}): any {
  return {
    outgoingRequest: null,
    incomingRequests: [],
    discoveredDevices: [],
    discoveredByChannel: { [ChannelKind.Network]: [], [ChannelKind.Bluetooth]: [] },
    discoveredWithPairedByChannel: { [ChannelKind.Network]: [], [ChannelKind.Bluetooth]: [] },
    pairCompletedAt: null,
    enterAddMode: jest.fn().mockResolvedValue(undefined),
    leaveAddMode: jest.fn().mockResolvedValue(undefined),
    requestPair: jest.fn().mockResolvedValue(undefined),
    acceptIncomingRequest: jest.fn().mockResolvedValue(true),
    rejectIncomingRequest: jest.fn().mockResolvedValue(undefined),
    submitPairCode: jest.fn().mockResolvedValue(true),
    cancelOutgoingRequest: jest.fn(),
    beginChannelSetup: jest.fn().mockResolvedValue(undefined),
    channelSetupStatus: jest.fn().mockResolvedValue(null),
    endChannelSetup: jest.fn().mockResolvedValue([]),
    ...overrides,
  };
}

function mountDialog(props: Record<string, unknown> = {}) {
  return mount(DeviceSetupDialog, {
    props: { open: true, ...props },
    global: { stubs: { Teleport: true } },
  });
}

async function flushUi() {
  for (let i = 0; i < 3; i += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

describe("DeviceSetupDialog, new device", () => {
  it("asks which channel before discovering anything", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(storeMock());
    const wrapper = mountDialog();
    await flushUi();

    expect(wrapper.find('[data-testid="pair-channel-network"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="pair-channel-bluetooth"]').exists()).toBe(true);
    // Nothing is discovered until a channel has been chosen -- the whole
    // point of asking out loud rather than inferring it.
    expect(wrapper.find('[data-testid="nearby-device-row"]').exists()).toBe(false);
  });

  /**
   * A peer visible over both channels appears in `discoveredDevices` only as
   * a Network entry -- that list is deduplicated with Network preferred. So
   * the dialog must read the per-channel lists instead: filtering the
   * deduplicated one by the chosen channel made the peer vanish the moment
   * Bluetooth was selected, and two devices on one LAN is the common case,
   * not an edge one.
   */
  it("still lists a peer over Bluetooth when it is also visible over the network", async () => {
    const peer = {
      device_id: "peer-1",
      hostname: "Pixel 8",
      addr: "AA:BB:CC:DD:EE:FF",
      discovery_port: 0,
      ws_port: null,
      last_seen_at: new Date().toISOString(),
      channel_kind: ChannelKind.Bluetooth,
    };
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        // Deduplicated list keeps only the Network entry, as the store does.
        discoveredDevices: [{ ...peer, channel_kind: ChannelKind.Network }],
        discoveredByChannel: { [ChannelKind.Network]: [{ ...peer, channel_kind: ChannelKind.Network }], [ChannelKind.Bluetooth]: [peer] },
      }),
    );

    const wrapper = mountDialog();
    await flushUi();
    await wrapper.find('[data-testid="pair-channel-bluetooth"]').trigger("click");
    await flushUi();

    const rows = wrapper.findAll('[data-testid="nearby-device-row"]');
    expect(rows).toHaveLength(1);
    expect(rows[0].attributes("data-channel-kind")).toBe(ChannelKind.Bluetooth);
  });

  it("enters add mode on open, because that is what makes this device discoverable", async () => {
    const store = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    mountDialog();
    await flushUi();

    expect(store.enterAddMode).toHaveBeenCalled();
  });

  /**
   * The code exists only after the other side accepts. Before acceptance
   * there is nothing to type, nothing to intercept and nothing to
   * shoulder-surf -- so the waiting screen must not show one.
   */
  it("shows no code while the request is merely pending", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        outgoingRequest: {
          request_id: "r1",
          to_device_id: "peer",
          to_hostname: "Pixel 8",
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() + 60_000).toISOString(),
          status: "pending",
          sender_code: null,
        },
      }),
    );
    const wrapper = mountDialog();
    await flushUi();

    expect(wrapper.text()).toContain("Waiting for Pixel 8");
    expect(wrapper.find('[data-testid="pair-code"]').exists()).toBe(false);
  });

  it("shows the six digits once a code exists", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        outgoingRequest: {
          request_id: "r1",
          to_device_id: "peer",
          to_hostname: "Pixel 8",
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() + 60_000).toISOString(),
          status: "awaiting_code",
          sender_code: "482915",
        },
      }),
    );
    const wrapper = mountDialog();
    await flushUi();

    const digits = wrapper.findAll('[data-testid="pair-code"]');
    expect(digits).toHaveLength(6);
    expect(digits.map((d) => d.text()).join("")).toBe("482915");
  });

  /**
   * Covered here rather than end-to-end on purpose.
   *
   * Declining is not currently sent to the requester: `rejectIncomingRequest`
   * only acknowledges the request locally, so `status: "rejected"` is
   * reachable today only when the *send itself* failed. Until the protocol
   * carries a decline (see the follow-up issue), this screen cannot happen
   * in production, and an e2e driving it would be asserting a state the
   * test itself had fabricated.
   *
   * What is worth pinning is that the screen says the right thing when the
   * state does arrive -- particularly that nothing was shared, which is the
   * reassurance the asker actually needs.
   */
  it("says the peer declined, and that nothing was shared", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        outgoingRequest: {
          request_id: "r1",
          to_device_id: "peer",
          to_hostname: "Pixel 8",
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() + 60_000).toISOString(),
          status: "rejected",
          sender_code: null,
        },
      }),
    );
    const wrapper = mountDialog();
    await flushUi();

    expect(wrapper.text()).toContain("Pixel 8 said no");
    expect(wrapper.text()).toContain("no code was created");
    expect(wrapper.find('[data-testid="pair-ask-again"]').exists()).toBe(true);
  });

  /**
   * An expired request covers both "nobody answered" and "they walked off"
   * -- expiry is the only signal this side has. What changes is whether a
   * code had already been issued, because a dead code someone is still
   * typing needs saying out loud.
   */
  it("names physical causes when the request runs out", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        outgoingRequest: {
          request_id: "r1",
          to_device_id: "peer",
          to_hostname: "Pixel 8",
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() - 1_000).toISOString(),
          status: "expired",
          sender_code: null,
        },
      }),
    );
    const wrapper = mountDialog();
    await flushUi();

    expect(wrapper.text()).toContain("The request ran out");
    expect(wrapper.text()).toContain("asleep");
  });

  it("warns that an already-issued code is dead once the request runs out", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        outgoingRequest: {
          request_id: "r1",
          to_device_id: "peer",
          to_hostname: "Pixel 8",
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() - 1_000).toISOString(),
          status: "expired",
          sender_code: "482915",
        },
      }),
    );
    const wrapper = mountDialog();
    await flushUi();

    expect(wrapper.text()).toContain("That code no longer works");
  });

  /**
   * Someone waiting on an answer outranks whatever this side was doing --
   * a request that expires unseen is the worst outcome of the ceremony.
   */
  it("interrupts with an incoming request whatever step it was showing", async () => {
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(
      storeMock({
        incomingRequests: [
          {
            request_id: "in-1",
            from_device_id: "peer",
            from_hostname: "Thinkpad",
            received_at: new Date().toISOString(),
            expires_at: new Date(Date.now() + 60_000).toISOString(),
            cooldown_until: null,
            via_bluetooth: false,
            from_bluetooth_address: null,
          },
        ],
      }),
    );
    const wrapper = mountDialog();
    await flushUi();

    expect(wrapper.text()).toContain("Thinkpad wants to pair");
    expect(wrapper.find('[data-testid="accept-incoming-request"]').exists()).toBe(true);
  });
});

describe("DeviceSetupDialog, known device (ADR-0008 D20)", () => {
  const known = { peerDeviceId: "peer-1", peerName: "Pixel 8", kind: ChannelKind.Bluetooth };

  beforeEach(() => jest.useFakeTimers());
  afterEach(() => jest.useRealTimers());

  it("skips the channel step and starts the setup search for the row's channel", async () => {
    const store = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    const wrapper = mountDialog(known);
    await flushUi();

    expect(wrapper.find('[data-testid="pair-channel-network"]').exists()).toBe(false);
    expect(store.beginChannelSetup).toHaveBeenCalledWith("peer-1", ChannelKind.Bluetooth);
    expect(wrapper.find('[data-testid="setup-peer-row"]').text()).toContain("Pixel 8");
  });

  it("keeps OK disabled until the channel is initialized on both devices", async () => {
    const store = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    const wrapper = mountDialog(known);
    await flushUi();
    expect(wrapper.find('[data-testid="setup-ok"]').attributes("disabled")).toBeDefined();

    store.channelSetupStatus.mockResolvedValue({
      helloAckedByPeer: true,
      ackedPeerHello: true,
      initialized: true,
    });
    jest.advanceTimersByTime(1_000);
    await flushUi();

    expect(wrapper.find('[data-testid="setup-ok"]').attributes("disabled")).toBeUndefined();
    await wrapper.find('[data-testid="setup-ok"]').trigger("click");
    await flushUi();

    expect(store.endChannelSetup).toHaveBeenCalledWith("peer-1", ChannelKind.Bluetooth, true);
    expect(wrapper.emitted("close")).toBeTruthy();
  });

  it("lets OK be pressed again when writing the channel failed", async () => {
    const store = storeMock({
      channelSetupStatus: jest.fn().mockResolvedValue({
        helloAckedByPeer: true,
        ackedPeerHello: true,
        initialized: true,
      }),
      endChannelSetup: jest
        .fn()
        .mockRejectedValueOnce(new Error("database is locked"))
        .mockResolvedValue([]),
    });
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    const warn = jest.spyOn(console, "warn").mockImplementation(() => {});
    const wrapper = mountDialog(known);
    await flushUi();
    jest.advanceTimersByTime(1_000);
    await flushUi();

    await wrapper.find('[data-testid="setup-ok"]').trigger("click");
    await flushUi();
    expect(wrapper.emitted("close")).toBeFalsy();

    await wrapper.find('[data-testid="setup-ok"]').trigger("click");
    await flushUi();
    expect(store.endChannelSetup).toHaveBeenCalledTimes(2);
    expect(wrapper.emitted("close")).toBeTruthy();
    warn.mockRestore();
  });

  it("leaves the Bluetooth half of add mode off when adding a Network channel", async () => {
    const store = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    mountDialog({ ...known, kind: ChannelKind.Network });
    await flushUi();
    expect(store.enterAddMode).toHaveBeenCalledWith({ bluetooth: false });
  });

  it("uses the Bluetooth half of add mode when adding a Bluetooth channel", async () => {
    const store = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    mountDialog(known);
    await flushUi();
    expect(store.enterAddMode).toHaveBeenCalledWith({ bluetooth: true });
  });

  it("ends the setup without switching on when closed", async () => {
    const store = storeMock();
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    const wrapper = mountDialog(known);
    await flushUi();

    await wrapper.find('[data-testid="setup-close"]').trigger("click");
    wrapper.unmount();
    await flushUi();

    expect(store.endChannelSetup).toHaveBeenCalledTimes(1);
    expect(store.endChannelSetup).toHaveBeenCalledWith("peer-1", ChannelKind.Bluetooth, false);
  });

  it("lists other devices in range, and offers the code when the peer asks for one", async () => {
    const nearby = (device_id: string, hostname: string) => ({
      device_id,
      hostname,
      addr: "AA:BB:CC:DD:EE:FF",
      discovery_port: 0,
      ws_port: null,
      last_seen_at: new Date().toISOString(),
      channel_kind: ChannelKind.Bluetooth,
    });
    // As the store has it: the known peer is paired, so only the list that
    // keeps paired devices contains it.
    const store = storeMock({
      discoveredByChannel: {
        [ChannelKind.Network]: [],
        [ChannelKind.Bluetooth]: [nearby("other", "Thinkpad")],
      },
      discoveredWithPairedByChannel: {
        [ChannelKind.Network]: [],
        [ChannelKind.Bluetooth]: [nearby("peer-1", "Pixel 8"), nearby("other", "Thinkpad")],
      },
    });
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    const wrapper = mountDialog(known);
    await flushUi();

    const others = wrapper.findAll('[data-testid="nearby-device-row"]');
    expect(others).toHaveLength(1);
    expect(others[0].text()).toContain("Thinkpad");

    await wrapper.find('[data-testid="setup-pair-with-code"]').trigger("click");
    expect(store.requestPair).toHaveBeenCalledWith(expect.objectContaining({ device_id: "peer-1" }));
  });

  // Closing while the begin still waits on the radio or a permission prompt
  // ran the end before the backend search existed; the search that started
  // afterwards must be ended too, not left running with no dialog.
  it("ends a setup search that finishes starting after the dialog closed", async () => {
    let resolveBegin: () => void = () => {};
    const store = storeMock({
      beginChannelSetup: jest.fn(
        () => new Promise<void>((resolve) => { resolveBegin = resolve; }),
      ),
    });
    (useDeviceStore as unknown as jest.Mock).mockReturnValue(store);
    const wrapper = mountDialog(known);
    await flushUi();

    wrapper.unmount();
    await flushUi();
    resolveBegin();
    await flushUi();

    expect(store.endChannelSetup).toHaveBeenCalledTimes(2);
    expect(store.endChannelSetup).toHaveBeenLastCalledWith("peer-1", ChannelKind.Bluetooth, false);
    jest.advanceTimersByTime(5_000);
    expect(store.channelSetupStatus).not.toHaveBeenCalled();
  });
});
