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

describe("device store channel rows (ADR-0008)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (invoke as unknown as jest.Mock).mockReset();
  });

  it("stores the backend's rows as they are, colour and problem included", async () => {
    const rows = [
      { kind: "network", state: "on", color: "green", problem: null },
      { kind: "bluetooth", state: "on", color: "orange", problem: "bluetooth_unavailable" },
    ];
    (invoke as unknown as jest.Mock).mockResolvedValueOnce(rows);

    const store = useDeviceStore();
    await store.refreshChannelStatuses("peer-1");

    expect(invoke).toHaveBeenCalledWith("device_connection_channel_statuses", {
      peerDeviceId: "peer-1",
    });
    expect(store.getChannelStatuses("peer-1")).toEqual(rows);
  });

  it("counts a device online only while one of its channels is green", async () => {
    const store = useDeviceStore();
    const device = { peer_device_id: "peer-1", last_seen_at: null } as never;

    (invoke as unknown as jest.Mock).mockResolvedValueOnce([
      { kind: "network", state: "on", color: "grey", problem: null },
      { kind: "bluetooth", state: "off", color: "off", problem: null },
    ]);
    await store.refreshChannelStatuses("peer-1");
    expect(store.isDeviceOnline(device)).toBe(false);

    (invoke as unknown as jest.Mock).mockResolvedValueOnce([
      { kind: "network", state: "on", color: "green", problem: null },
      { kind: "bluetooth", state: "off", color: "off", problem: null },
    ]);
    await store.refreshChannelStatuses("peer-1");
    expect(store.isDeviceOnline(device)).toBe(true);
  });

  it("drives the channel setup through its three commands", async () => {
    (invoke as unknown as jest.Mock).mockResolvedValue(undefined);
    const store = useDeviceStore();

    await store.beginChannelSetup("peer-1", "bluetooth");
    (invoke as unknown as jest.Mock).mockResolvedValueOnce({
      helloAckedByPeer: true,
      ackedPeerHello: true,
      initialized: true,
    });
    const status = await store.channelSetupStatus("peer-1", "bluetooth");
    await store.endChannelSetup("peer-1", "bluetooth", true);

    expect(status?.initialized).toBe(true);
    expect(invoke).toHaveBeenCalledWith("device_connection_begin_channel_setup", {
      peerDeviceId: "peer-1",
      kind: "bluetooth",
    });
    expect(invoke).toHaveBeenCalledWith("device_connection_channel_setup_status", {
      peerDeviceId: "peer-1",
      kind: "bluetooth",
    });
    expect(invoke).toHaveBeenCalledWith(
      "device_connection_end_channel_setup",
      expect.objectContaining({ peerDeviceId: "peer-1", kind: "bluetooth", switchOn: true }),
    );
  });
});
