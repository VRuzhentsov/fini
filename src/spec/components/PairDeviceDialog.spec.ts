import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import PairDeviceDialog from "../../components/SettingsView/PairDeviceDialog.vue";
import { useDeviceStore } from "../../stores/device";

jest.mock("../../stores/device", () => ({
  useDeviceStore: jest.fn(),
}));

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function storeMock(overrides: Record<string, unknown> = {}): any {
  return {
    outgoingRequest: null,
    incomingRequests: [],
    discoveredDevices: [],
    pairCompletedAt: null,
    enterAddMode: jest.fn().mockResolvedValue(undefined),
    leaveAddMode: jest.fn().mockResolvedValue(undefined),
    requestPair: jest.fn().mockResolvedValue(undefined),
    acceptIncomingRequest: jest.fn().mockResolvedValue(true),
    rejectIncomingRequest: jest.fn().mockResolvedValue(undefined),
    submitPairCode: jest.fn().mockResolvedValue(true),
    cancelOutgoingRequest: jest.fn(),
    ...overrides,
  };
}

function mountDialog() {
  return mount(PairDeviceDialog, {
    props: { open: true },
    global: { stubs: { Teleport: true } },
  });
}

async function flushUi() {
  for (let i = 0; i < 3; i += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

describe("PairDeviceDialog", () => {
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
