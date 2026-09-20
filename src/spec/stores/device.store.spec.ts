import { invoke } from "@tauri-apps/api/core";
import { createPinia, setActivePinia } from "pinia";
import { useDeviceStore } from "../../stores/device";

jest.mock("@tauri-apps/api/core", () => ({
  invoke: jest.fn(),
}));

describe("device store sync status", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (invoke as unknown as jest.Mock).mockReset();
  });

  it("updates last synced timestamp from status response", async () => {
    (invoke as unknown as jest.Mock).mockResolvedValueOnce({
      peer_device_id: "peer-1",
      mapped_space_ids: ["1", "2"],
      last_synced_at: "2026-04-07T13:20:00Z",
      last_synced_at_by_space: {
        "1": "2026-04-07T13:15:00Z",
        "2": "2026-04-07T13:20:00Z",
      },
      pending_event_count: 0,
      outbox_event_count: 10,
      acked_event_count: 10,
      seen_event_count: 10,
      tombstone_count: 0,
    });

    const store = useDeviceStore();
    await store.refreshSpaceSyncStatus("peer-1");

    expect(store.getLastSyncedAt("peer-1")).toBe("2026-04-07T13:20:00Z");
    expect(store.getLastSyncedAtForSpace("peer-1", "1")).toBe("2026-04-07T13:15:00Z");
    expect(store.getLastSyncedAtForSpace("peer-1", "2")).toBe("2026-04-07T13:20:00Z");
  });

  it("stores incoming sync requests until explicitly approved", async () => {
    (invoke as unknown as jest.Mock)
      .mockResolvedValueOnce([
        {
          from_device_id: "peer-1",
          mapped_space_ids: ["1", "2"],
          custom_spaces: [{ space_id: "space-9", name: "Shared" }],
          sent_at: "2026-04-11T20:00:00Z",
        },
      ])
      .mockResolvedValueOnce(["1"]);

    const store = useDeviceStore();
    await store.loadMappedSpaces("peer-1");

    expect(store.getPendingSpaceSyncRequest("peer-1")).toEqual({
      peer_device_id: "peer-1",
      mapped_space_ids: ["1", "2"],
      custom_spaces: [{ space_id: "space-9", name: "Shared" }],
      sent_at: "2026-04-11T20:00:00Z",
    });

    const invokedCommands = (invoke as unknown as jest.Mock).mock.calls.map(([command]) => command);
    expect(invokedCommands).not.toContain("space_sync_apply_remote_mappings");
  });

  it("applies a pending sync request only after approval", async () => {
    (invoke as unknown as jest.Mock)
      .mockResolvedValueOnce([
        {
          from_device_id: "peer-1",
          mapped_space_ids: ["1"],
          custom_spaces: [],
          sent_at: "2026-04-11T20:00:00Z",
        },
      ])
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce({
        mapped_space_ids: ["1"],
        unresolved_custom_spaces: [],
      })
      .mockResolvedValueOnce({
        peer_device_id: "peer-1",
        mapped_space_ids: ["1"],
        pending_event_count: 1,
        outbox_event_count: 1,
        acked_event_count: 0,
        seen_event_count: 0,
        tombstone_count: 0,
      })
      .mockResolvedValueOnce({
        sent_events: 0,
        applied_events: 0,
        received_acks: 0,
        peers: [],
        ticked_at: "2026-04-11T20:00:10Z",
      });

    const store = useDeviceStore();
    await store.loadMappedSpaces("peer-1");
    await store.approvePendingSpaceSyncRequest("peer-1");

    expect((invoke as unknown as jest.Mock).mock.calls).toContainEqual([
      "space_sync_apply_remote_mappings",
      {
        peerDeviceId: "peer-1",
        mappedSpaceIds: ["1"],
        customSpaces: [],
      },
    ]);
    expect(store.getPendingSpaceSyncRequest("peer-1")).toBeNull();
    expect(store.getMappedSpaceIds("peer-1")).toEqual(["1"]);
  });

  it("ignores empty incoming sync requests", async () => {
    (invoke as unknown as jest.Mock)
      .mockResolvedValueOnce([
        {
          from_device_id: "peer-1",
          mapped_space_ids: [],
          custom_spaces: [],
          sent_at: "2026-04-11T20:00:00Z",
        },
      ])
      .mockResolvedValueOnce([]);

    const store = useDeviceStore();
    await store.loadMappedSpaces("peer-1");

    expect(store.getPendingSpaceSyncRequest("peer-1")).toBeNull();
    expect(store.listPendingSpaceSyncRequests()).toEqual([]);
  });

  it("removes a pending sync request before approval completes", async () => {
    let resolveApply!: (value: { mapped_space_ids: string[]; unresolved_custom_spaces: [] }) => void;
    const applyPromise = new Promise<{ mapped_space_ids: string[]; unresolved_custom_spaces: [] }>((resolve) => {
      resolveApply = resolve;
    });

    (invoke as unknown as jest.Mock)
      .mockResolvedValueOnce([
        {
          from_device_id: "peer-1",
          mapped_space_ids: ["1"],
          custom_spaces: [],
          sent_at: "2026-04-11T20:00:00Z",
        },
      ])
      .mockResolvedValueOnce([])
      .mockImplementationOnce(() => applyPromise)
      .mockResolvedValueOnce({
        peer_device_id: "peer-1",
        mapped_space_ids: ["1"],
        pending_event_count: 1,
        outbox_event_count: 1,
        acked_event_count: 0,
        seen_event_count: 0,
        tombstone_count: 0,
      })
      .mockResolvedValueOnce({
        sent_events: 0,
        applied_events: 0,
        received_acks: 0,
        peers: [],
        ticked_at: "2026-04-11T20:00:10Z",
      });

    const store = useDeviceStore();
    await store.loadMappedSpaces("peer-1");

    const approvalPromise = store.approvePendingSpaceSyncRequest("peer-1");

    expect(store.getPendingSpaceSyncRequest("peer-1")).toBeNull();
    expect(store.listPendingSpaceSyncRequests()).toEqual([]);

    resolveApply({
      mapped_space_ids: ["1"],
      unresolved_custom_spaces: [],
    });

    await approvalPromise;
    expect(store.getMappedSpaceIds("peer-1")).toEqual(["1"]);
  });

  it("does not reopen the approval dialog for a confirmation snapshot", async () => {
    (invoke as unknown as jest.Mock)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce(["1"])
      .mockResolvedValueOnce([
        {
          from_device_id: "peer-1",
          mapped_space_ids: ["1"],
          custom_spaces: [],
          sent_at: "2026-04-11T20:00:10Z",
        },
      ])
      .mockResolvedValueOnce(["1"])
      .mockResolvedValueOnce(["1"]);

    const store = useDeviceStore();

    await store.loadMappedSpaces("peer-1");
    await store.loadMappedSpaces("peer-1");

    expect(store.getMappedSpaceIds("peer-1")).toEqual(["1"]);
    expect(store.getPendingSpaceSyncRequest("peer-1")).toBeNull();
    expect(store.listPendingSpaceSyncRequests()).toEqual([]);
  });
});

describe("device store channel liveness", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (invoke as unknown as jest.Mock).mockReset();
  });

  // Regression test for a P1 review finding: the lightweight live-poll
  // (`refreshLiveConnectedState`, backed by
  // `device_connection_channel_liveness`) used to leave `state`/`code`
  // frozen at whatever the last full `refreshChannelStatuses` poll saw --
  // wrong once green/amber became a continuously-reproven ping/ack proof
  // that can lapse or complete on its own.
  //
  // The poll deliberately carries no `primary`: the star is the person's
  // stored choice, and letting a liveness signal rewrite it is how a
  // setting appears to move by itself.
  it("refreshes the ping/ack code on the lightweight poll, and never the star", async () => {
    (invoke as unknown as jest.Mock).mockResolvedValueOnce([
      {
        kind: "network",
        configured: true,
        enabled: true,
        primary: true,
        address: null,
        state: { state: "configured", code: { code: "awaiting_first_ack" } },
      },
      {
        kind: "bluetooth",
        configured: true,
        enabled: false,
        primary: false,
        address: null,
        state: { state: "unconfigured", code: { code: "bluetooth_disabled" } },
      },
    ]);

    const store = useDeviceStore();
    await store.refreshChannelStatuses("peer-1");
    expect(store.getChannelStatuses("peer-1")[0].state).toEqual({
      state: "configured",
      code: { code: "awaiting_first_ack" },
    });

    (invoke as unknown as jest.Mock).mockResolvedValueOnce([
      { kind: "network", connected: true, code: null },
      { kind: "bluetooth", connected: false, code: null },
    ]);
    await store.refreshLiveConnectedState("peer-1");

    const [network, bluetooth] = store.getChannelStatuses("peer-1");
    expect(network.state).toEqual({ state: "configured", code: null });
    expect(network.primary).toBe(true);
    // Switched off: there is no live state to patch, because the row says
    // what the person chose rather than what a radio is doing.
    expect(bluetooth.state).toEqual({
      state: "unconfigured",
      code: { code: "bluetooth_disabled" },
    });
  });

  // Regression test for a P2 review finding: a configured-but-not-yet-
  // connected channel (dial in flight, no session claimed) must report
  // "connecting", not "awaiting_first_ack" -- that code specifically means
  // a session *is* claimed and the ping/ack proof just hasn't completed
  // its first round, which the UI renders as "Connected -- waiting for the
  // first ping/ack exchange." Reporting that for an entirely unconnected
  // row (e.g. presenced but the WebSocket port is unreachable) would be
  // misleading.
  it("reports connecting, not awaiting_first_ack, for a configured row with no session yet", async () => {
    (invoke as unknown as jest.Mock).mockResolvedValueOnce([
      {
        kind: "network",
        configured: true,
        enabled: true,
        primary: false,
        address: null,
        state: { state: "configured", code: null },
      },
      {
        kind: "bluetooth",
        configured: true,
        enabled: false,
        primary: false,
        address: null,
        state: { state: "unconfigured", code: { code: "bluetooth_disabled" } },
      },
    ]);

    const store = useDeviceStore();
    await store.refreshChannelStatuses("peer-1");

    (invoke as unknown as jest.Mock).mockResolvedValueOnce([
      { kind: "network", connected: false, code: null },
      { kind: "bluetooth", connected: false, code: null },
    ]);
    await store.refreshLiveConnectedState("peer-1");

    const [network] = store.getChannelStatuses("peer-1");
    expect(network.state).toEqual({ state: "configured", code: { code: "connecting" } });
  });
});
