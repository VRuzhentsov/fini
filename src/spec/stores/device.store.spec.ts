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
      pending_event_count: 0,
      outbox_event_count: 10,
      acked_event_count: 10,
      seen_event_count: 10,
      tombstone_count: 0,
    });

    const store = useDeviceStore();
    await store.refreshSpaceSyncStatus("peer-1");

    expect(store.getLastSyncedAt("peer-1")).toBe("2026-04-07T13:20:00Z");
  });
});
