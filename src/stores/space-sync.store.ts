import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import { ref } from "vue";
import { FiniEventType, onFiniEvent } from "../utils/fini-event";
import { useDeviceStore } from "./device.store";

// The store of SpaceSyncService (ADR-0010 D16): the spaces each pair syncs,
// how far the sync is, and the requests to sync a space.

export interface SpaceSyncStatus {
  peer_device_id: string | null;
  mapped_space_ids: string[];
  last_synced_at?: string | null;
  last_synced_at_by_space?: Record<string, string | null>;
  end_of_sync_at_by_space?: Record<string, string | null>;
  pending_event_count: number;
  outbox_event_count: number;
  acked_event_count: number;
  seen_event_count: number;
  tombstone_count: number;
}

// One change waiting to reach a peer. `title` is given by the backend only
// for a Quest or a Space; the frontend names the other entity types.
export interface SyncQueueEntry {
  entity_type: string;
  entity_id: string;
  space_id: string;
  title: string | null;
}

export interface SyncQueueSummary {
  peer_device_id: string;
  // The real total; `entries` is capped by the backend.
  pending_count: number;
  entries: SyncQueueEntry[];
  last_acked_at: string | null;
}

export interface SpaceSyncTickPeer {
  peer_device_id: string;
  sent_events: number;
  received_events: number;
  acked_events: number;
  transferred_objects: number;
  last_transfer_at: string;
}

export interface SpaceSyncTickResult {
  sent_events: number;
  applied_events: number;
  received_acks: number;
  peers: SpaceSyncTickPeer[];
  ticked_at: string;
}

export interface CustomSpaceDescriptor {
  space_id: string;
  name: string;
}

export interface IncomingSpaceMappingUpdate {
  from_device_id: string;
  mapped_space_ids: string[];
  custom_spaces: CustomSpaceDescriptor[];
  sent_at: string;
}

export interface PendingSpaceSyncRequest {
  peer_device_id: string;
  mapped_space_ids: string[];
  custom_spaces: CustomSpaceDescriptor[];
  sent_at: string;
}

export interface UnresolvedCustomSpace {
  space_id: string;
  name: string;
}

export interface SpaceMappingApplyResult {
  mapped_space_ids: string[];
  unresolved_custom_spaces: UnresolvedCustomSpace[];
}

export const useSpaceSyncStore = defineStore("space-sync", () => {
  const deviceStore = useDeviceStore();
  const mappedSpaceIdsByPeer = ref<Record<string, string[]>>({});
  const unresolvedCustomSpacesByPeer = ref<Record<string, UnresolvedCustomSpace[]>>({});
  const pendingSpaceSyncRequestsByPeer = ref<Record<string, PendingSpaceSyncRequest>>({});
  const syncStatusByPeer = ref<Record<string, SpaceSyncStatus | null>>({});
  const lastSyncedAtByPeer = ref<Record<string, string | null>>({});
  const lastSyncedAtByPeerSpace = ref<Record<string, Record<string, string | null>>>({});
  const syncingByPeer = ref<Record<string, boolean>>({});
  const syncQueueByPeer = ref<Record<string, SyncQueueSummary | null>>({});
  const lastAppliedSyncAt = ref<string | null>(null);

  function pairedDeviceIds(): string[] {
    return deviceStore.pairedDevices.map((device) => device.peer_device_id);
  }

  function getMappedSpaceIds(peerDeviceId: string): string[] {
    return mappedSpaceIdsByPeer.value[peerDeviceId] ?? [];
  }

  function getUnresolvedCustomSpaces(peerDeviceId: string): UnresolvedCustomSpace[] {
    return unresolvedCustomSpacesByPeer.value[peerDeviceId] ?? [];
  }

  function getPendingSpaceSyncRequest(peerDeviceId: string): PendingSpaceSyncRequest | null {
    return (
      Object.values(pendingSpaceSyncRequestsByPeer.value).find((request) => request.peer_device_id === peerDeviceId) ??
      null
    );
  }

  function listPendingSpaceSyncRequests(): PendingSpaceSyncRequest[] {
    return Object.values(pendingSpaceSyncRequestsByPeer.value).sort(
      (a, b) => b.sent_at.localeCompare(a.sent_at) || a.peer_device_id.localeCompare(b.peer_device_id),
    );
  }

  function getSpaceSyncStatus(peerDeviceId: string): SpaceSyncStatus | null {
    return syncStatusByPeer.value[peerDeviceId] ?? null;
  }

  function getLastSyncedAt(peerDeviceId: string): string | null {
    return lastSyncedAtByPeer.value[peerDeviceId] ?? null;
  }

  function getLastSyncedAtBySpace(peerDeviceId: string): Record<string, string | null> {
    const bySpace = lastSyncedAtByPeerSpace.value[peerDeviceId];
    if (bySpace) return bySpace;
    const fallback = lastSyncedAtByPeer.value[peerDeviceId];
    if (!fallback) return {};
    const mapped = mappedSpaceIdsByPeer.value[peerDeviceId] ?? [];
    return Object.fromEntries(mapped.map((spaceId) => [spaceId, fallback]));
  }

  function getLastSyncedAtForSpace(peerDeviceId: string, spaceId: string): string | null {
    return getLastSyncedAtBySpace(peerDeviceId)[spaceId] ?? null;
  }

  function isSyncingPeer(peerDeviceId: string): boolean {
    return syncingByPeer.value[peerDeviceId] ?? false;
  }

  function getSyncQueue(peerDeviceId: string): SyncQueueSummary | null {
    return syncQueueByPeer.value[peerDeviceId] ?? null;
  }

  async function loadMappedSpaces(peerDeviceId: string): Promise<string[]> {
    await takeSpaceMappingUpdates();
    try {
      const mapped = await invoke<string[]>("space_sync_list_mappings", { peerDeviceId });
      mappedSpaceIdsByPeer.value[peerDeviceId] = mapped;
      return mapped;
    } catch (error) {
      console.warn("[space-sync] failed to load mappings", error);
      mappedSpaceIdsByPeer.value[peerDeviceId] = [];
      return [];
    }
  }

  async function saveMappedSpaces(peerDeviceId: string, mappedSpaceIds: string[]): Promise<string[]> {
    const mapped = await invoke<string[]>("space_sync_update_mappings", { peerDeviceId, mappedSpaceIds });
    mappedSpaceIdsByPeer.value[peerDeviceId] = mapped;
    void refreshSpaceSyncStatus(peerDeviceId);
    return mapped;
  }

  async function resolveCustomSpaceMapping(
    peerDeviceId: string,
    remoteSpaceId: string,
    resolutionMode: "create_new" | "use_existing",
    existingSpaceId?: string,
    remoteSpaceName?: string,
  ) {
    const unresolvedBefore = unresolvedCustomSpacesByPeer.value[peerDeviceId] ?? [];
    const result = await invoke<SpaceMappingApplyResult>("space_sync_resolve_custom_space_mapping", {
      peerDeviceId,
      remoteSpaceId,
      remoteSpaceName,
      resolutionMode,
      existingSpaceId,
    });
    mappedSpaceIdsByPeer.value[peerDeviceId] = result.mapped_space_ids;
    const fallbackRemaining = unresolvedBefore.filter((space) => space.space_id !== remoteSpaceId);
    const unresolvedFromBackend = result.unresolved_custom_spaces.filter((space) => space.space_id !== remoteSpaceId);
    unresolvedCustomSpacesByPeer.value[peerDeviceId] =
      unresolvedFromBackend.length > 0 ? unresolvedFromBackend : fallbackRemaining;
    void refreshSpaceSyncStatus(peerDeviceId);
    return result;
  }

  async function approvePendingSpaceSyncRequest(peerDeviceId: string, spaceId?: string): Promise<SpaceMappingApplyResult> {
    const pendingEntry = Object.entries(pendingSpaceSyncRequestsByPeer.value).find(
      ([, request]) => request.peer_device_id === peerDeviceId && (!spaceId || request.mapped_space_ids.includes(spaceId)),
    );
    const pendingRequest = pendingEntry?.[1];
    if (!pendingEntry || !pendingRequest) {
      throw new Error(`No pending space sync request for ${peerDeviceId}`);
    }
    delete pendingSpaceSyncRequestsByPeer.value[pendingEntry[0]];
    try {
      const result = await invoke<SpaceMappingApplyResult>("space_sync_apply_remote_mappings", {
        peerDeviceId,
        mappedSpaceIds: pendingRequest.mapped_space_ids,
        customSpaces: pendingRequest.custom_spaces,
      });
      mappedSpaceIdsByPeer.value[peerDeviceId] = result.mapped_space_ids;
      unresolvedCustomSpacesByPeer.value[peerDeviceId] = result.unresolved_custom_spaces;
      void refreshSpaceSyncStatus(peerDeviceId);
      return result;
    } catch (error) {
      pendingSpaceSyncRequestsByPeer.value[pendingEntry[0]] = pendingRequest;
      throw error;
    }
  }

  async function refreshSpaceSyncStatus(peerDeviceId: string): Promise<SpaceSyncStatus | null> {
    try {
      const status = await invoke<SpaceSyncStatus>("space_sync_status", { peerDeviceId });
      syncStatusByPeer.value[peerDeviceId] = status;
      if (status.last_synced_at_by_space) {
        lastSyncedAtByPeerSpace.value[peerDeviceId] = { ...status.last_synced_at_by_space };
      } else {
        delete lastSyncedAtByPeerSpace.value[peerDeviceId];
      }
      if (status.last_synced_at) {
        const current = lastSyncedAtByPeer.value[peerDeviceId];
        const currentTs = current ? Date.parse(current) : Number.NaN;
        const nextTs = Date.parse(status.last_synced_at);
        if (!current || Number.isNaN(currentTs) || (!Number.isNaN(nextTs) && nextTs >= currentTs)) {
          lastSyncedAtByPeer.value[peerDeviceId] = status.last_synced_at;
        }
      }
      return status;
    } catch (error) {
      console.warn("[space-sync] failed to load status", error);
      syncStatusByPeer.value[peerDeviceId] = null;
      return null;
    }
  }

  // The Device page's sync queue: "has my data synced?". It also gives the
  // titles behind the count, so it is read only while that page is open.
  async function refreshSyncQueue(peerDeviceId: string): Promise<SyncQueueSummary | null> {
    try {
      const summary = await invoke<SyncQueueSummary>("space_sync_queue_summary", { peerDeviceId });
      syncQueueByPeer.value = { ...syncQueueByPeer.value, [peerDeviceId]: summary };
      return summary;
    } catch (error) {
      console.warn("[space-sync] failed to load the sync queue", error);
      return null;
    }
  }

  // One sync pass now. The backend's keeper runs sync by itself; this is
  // for a person's act, and for the first read after the app starts.
  async function runSpaceSyncTick() {
    const peerIds = pairedDeviceIds();
    for (const peerId of peerIds) syncingByPeer.value[peerId] = true;
    try {
      const tick = await invoke<SpaceSyncTickResult>("space_sync_tick");
      for (const peer of tick.peers) {
        if (peer.transferred_objects > 0) lastSyncedAtByPeer.value[peer.peer_device_id] = peer.last_transfer_at;
      }
      if (tick.applied_events > 0) lastAppliedSyncAt.value = tick.ticked_at;
      for (const peerId of peerIds) void refreshSpaceSyncStatus(peerId);
    } catch (error) {
      console.warn("[space-sync] tick failed", error);
    } finally {
      for (const peerId of peerIds) syncingByPeer.value[peerId] = false;
    }
  }

  // The requests to sync a space that a peer sent, once each.
  async function takeSpaceMappingUpdates() {
    try {
      const updates = await invoke<IncomingSpaceMappingUpdate[]>("space_sync_take_mapping_updates");
      for (const update of updates) {
        let currentMapped = mappedSpaceIdsByPeer.value[update.from_device_id] ?? [];
        const isEmptySnapshot = update.mapped_space_ids.length === 0 && update.custom_spaces.length === 0;
        let isAlreadyMapped = update.mapped_space_ids.every((spaceId) => currentMapped.includes(spaceId));
        if (isAlreadyMapped) {
          try {
            currentMapped = await invoke<string[]>("space_sync_list_mappings", { peerDeviceId: update.from_device_id });
            mappedSpaceIdsByPeer.value[update.from_device_id] = currentMapped;
            isAlreadyMapped = update.mapped_space_ids.every((spaceId) => currentMapped.includes(spaceId));
          } catch {
            // The store's copy stands until the next request.
          }
        }
        if (isEmptySnapshot) {
          delete pendingSpaceSyncRequestsByPeer.value[update.from_device_id];
          continue;
        }
        if (isAlreadyMapped) continue;
        const requestedSpaceId = update.mapped_space_ids[0];
        if (!requestedSpaceId) continue;
        pendingSpaceSyncRequestsByPeer.value[`${update.from_device_id}:${requestedSpaceId}`] = {
          peer_device_id: update.from_device_id,
          mapped_space_ids: [...update.mapped_space_ids],
          custom_spaces: [...update.custom_spaces],
          sent_at: update.sent_at,
        };
      }
    } catch (error) {
      console.warn("[space-sync] failed to take the mapping requests", error);
    }
  }

  // A pair was removed: forget what this store holds for it.
  function forgetPeer(peerDeviceId: string) {
    delete mappedSpaceIdsByPeer.value[peerDeviceId];
    delete unresolvedCustomSpacesByPeer.value[peerDeviceId];
    delete pendingSpaceSyncRequestsByPeer.value[peerDeviceId];
    delete syncStatusByPeer.value[peerDeviceId];
    delete lastSyncedAtByPeer.value[peerDeviceId];
    delete lastSyncedAtByPeerSpace.value[peerDeviceId];
    delete syncingByPeer.value[peerDeviceId];
  }

  let started = false;

  // The backend tells this store when sync data changed (ADR-0010 D9); the
  // store reads it again then, with no polling.
  async function start() {
    if (started) return;
    started = true;
    await onFiniEvent((event) => {
      if (event.type === FiniEventType.SpaceDataChanged) {
        lastAppliedSyncAt.value = new Date().toISOString();
        for (const peerId of pairedDeviceIds()) void refreshSpaceSyncStatus(peerId);
      } else if (event.type === FiniEventType.SyncQueueChanged) {
        for (const peerId of pairedDeviceIds()) void refreshSpaceSyncStatus(peerId);
        // The queues that a Device page shows.
        for (const peerId of Object.keys(syncQueueByPeer.value)) void refreshSyncQueue(peerId);
      } else if (event.type === FiniEventType.SpaceMappingRequestsChanged) {
        void takeSpaceMappingUpdates();
      } else if (event.type === FiniEventType.PairedDevicesChanged) {
        // A removed pair: forget what this store held for it.
        const paired = new Set(pairedDeviceIds());
        for (const peerId of Object.keys(syncStatusByPeer.value)) {
          if (!paired.has(peerId)) forgetPeer(peerId);
        }
      }
    });
    void takeSpaceMappingUpdates();
    // One pass at app start, for the first read of the sync status.
    void runSpaceSyncTick();
  }

  return {
    mappedSpaceIdsByPeer,
    unresolvedCustomSpacesByPeer,
    syncStatusByPeer,
    lastSyncedAtByPeer,
    lastSyncedAtByPeerSpace,
    syncingByPeer,
    lastAppliedSyncAt,
    start,
    loadMappedSpaces,
    saveMappedSpaces,
    resolveCustomSpaceMapping,
    approvePendingSpaceSyncRequest,
    getMappedSpaceIds,
    getUnresolvedCustomSpaces,
    getPendingSpaceSyncRequest,
    listPendingSpaceSyncRequests,
    refreshSpaceSyncStatus,
    getSpaceSyncStatus,
    getLastSyncedAt,
    getLastSyncedAtBySpace,
    getLastSyncedAtForSpace,
    getSyncQueue,
    refreshSyncQueue,
    isSyncingPeer,
    runSpaceSyncTick,
    takeSpaceMappingUpdates,
    forgetPeer,
  };
});
