import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { shortUuid } from "../utils/shortUuid";

export interface PairedDevice {
  peer_device_id: string;
  display_name: string;
  paired_at: string;
  last_seen_at: string | null;
  pair_state: string;
}

export interface DiscoveredDevice {
  device_id: string;
  hostname: string;
  addr: string;
  ws_port: number | null;
  last_seen_at: string;
}

export interface IncomingPairRequest {
  request_id: string;
  from_device_id: string;
  from_hostname: string;
  created_at: string;
  expires_at: string;
  attempts: number;
  cooldown_until: string | null;
}

export interface OutgoingPairRequest {
  request_id: string;
  to_device_id: string;
  to_hostname: string;
  created_at: string;
  expires_at: string;
  status: "pending" | "awaiting_code" | "expired" | "paired" | "rejected";
  sender_code: string | null;
}

export interface DeviceConnectionDebugStatus {
  add_mode_enabled: boolean;
  worker_started: boolean;
  tx_count: number;
  rx_count: number;
  discovered_count: number;
  incoming_request_count: number;
  outgoing_code_count: number;
  last_broadcast_at: string | null;
  last_error: string | null;
  discovery_port: number;
}

export interface PairCodeUpdate {
  request_id: string;
  code: string;
  accepted_at: string;
}

export interface PairCompletionUpdate {
  request_id: string;
  from_device_id: string;
  from_hostname: string;
  paired_at: string;
}

export interface SpaceSyncStatus {
  peer_device_id: string | null;
  mapped_space_ids: string[];
  last_synced_at?: string | null;
  pending_event_count: number;
  outbox_event_count: number;
  acked_event_count: number;
  seen_event_count: number;
  tombstone_count: number;
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

export interface IncomingSpaceMappingUpdate {
  from_device_id: string;
  mapped_space_ids: string[];
  custom_spaces: CustomSpaceDescriptor[];
  sent_at: string;
}

export interface CustomSpaceDescriptor {
  space_id: string;
  name: string;
}

export interface UnresolvedCustomSpace {
  space_id: string;
  name: string;
}

export interface SpaceMappingApplyResult {
  mapped_space_ids: string[];
  unresolved_custom_spaces: UnresolvedCustomSpace[];
}

interface DevicePairRequestInput {
  request_id: string;
  to_device_id: string;
  to_addr: string;
}

interface DevicePairRequestAckInput {
  request_id: string;
}

interface LocalDeviceIdentity {
  device_id: string;
  hostname: string;
}

export const ADD_MODE_DISCOVERY_INTERVAL_MS = 5_000;
const ADD_MODE_POLL_INTERVAL_MS = 1_000;
const PRESENCE_POLL_INTERVAL_MS = 15_000;
const MAPPING_UPDATE_POLL_INTERVAL_MS = 3_000;
const NORMAL_HEARTBEAT_MS = 60_000;
const OFFLINE_AFTER_MISSED_HEARTBEATS_MS = NORMAL_HEARTBEAT_MS * 2;
const REQUEST_TTL_MS = 60_000;

function nowIso(): string {
  return new Date().toISOString();
}

function randomId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `${Date.now().toString(16)}-${Math.random().toString(16).slice(2, 10)}`;
}

export const useDeviceStore = defineStore("device", () => {
  const identity = ref<LocalDeviceIdentity>({
    device_id: randomId(),
    hostname: "fini-device",
  });
  const pairedDevices = ref<PairedDevice[]>([]);
  const discoveredDevices = ref<DiscoveredDevice[]>([]);
  const incomingRequests = ref<IncomingPairRequest[]>([]);
  const outgoingRequest = ref<OutgoingPairRequest | null>(null);
  const addModeEnabled = ref(false);
  const addModeLastRefreshAt = ref<string | null>(null);
  const debugStatus = ref<DeviceConnectionDebugStatus | null>(null);
  const pairCompletedAt = ref<string | null>(null);
  const mappedSpaceIdsByPeer = ref<Record<string, string[]>>({});
  const unresolvedCustomSpacesByPeer = ref<Record<string, UnresolvedCustomSpace[]>>({});
  const syncStatusByPeer = ref<Record<string, SpaceSyncStatus | null>>({});
  const lastSyncedAtByPeer = ref<Record<string, string | null>>({});
  const syncingByPeer = ref<Record<string, boolean>>({});
  const lastAppliedSyncAt = ref<string | null>(null);
  const incomingExpectedCode = ref<Record<string, string>>({});
  const incomingAttemptCount = ref<Record<string, number>>({});
  const incomingCooldownUntil = ref<Record<string, string | null>>({});

  let discoveryTimer: ReturnType<typeof setInterval> | null = null;
  let presenceTimer: ReturnType<typeof setInterval> | null = null;
  let mappingUpdateTimer: ReturnType<typeof setInterval> | null = null;

  const outgoingRequestSecondsLeft = computed(() => {
    if (!outgoingRequest.value) return 0;
    const expiresAt = Date.parse(outgoingRequest.value.expires_at);
    const diff = expiresAt - Date.now();
    return diff > 0 ? Math.ceil(diff / 1000) : 0;
  });

  const pairedDeviceIds = computed(() => new Set(pairedDevices.value.map((d) => d.peer_device_id)));

  async function loadPairedDevices() {
    try {
      pairedDevices.value = await invoke<PairedDevice[]>("device_connection_get_paired_devices");
    } catch (error) {
      console.warn("[device-connection] failed to load paired devices", error);
    }
  }

  async function savePairedDevice(deviceId: string, displayName: string) {
    try {
      await invoke<PairedDevice>("device_connection_save_paired_device", {
        peerDeviceId: deviceId,
        displayName,
      });
      await loadPairedDevices();
    } catch (error) {
      console.warn("[device-connection] failed to save paired device", error);
    }
  }

  function getMappedSpaceIds(peerDeviceId: string): string[] {
    return mappedSpaceIdsByPeer.value[peerDeviceId] ?? [];
  }

  function getUnresolvedCustomSpaces(peerDeviceId: string): UnresolvedCustomSpace[] {
    return unresolvedCustomSpacesByPeer.value[peerDeviceId] ?? [];
  }

  function getSpaceSyncStatus(peerDeviceId: string): SpaceSyncStatus | null {
    return syncStatusByPeer.value[peerDeviceId] ?? null;
  }

  function getLastSyncedAt(peerDeviceId: string): string | null {
    return lastSyncedAtByPeer.value[peerDeviceId] ?? null;
  }

  function isSyncingPeer(peerDeviceId: string): boolean {
    return syncingByPeer.value[peerDeviceId] ?? false;
  }

  async function loadMappedSpaces(peerDeviceId: string): Promise<string[]> {
    await consumeSpaceMappingUpdates();

    try {
      const mapped = await invoke<string[]>("space_sync_list_mappings", {
        peerDeviceId,
      });
      mappedSpaceIdsByPeer.value[peerDeviceId] = mapped;
      return mapped;
    } catch (error) {
      console.warn("[space-sync] failed to load mappings", error);
      mappedSpaceIdsByPeer.value[peerDeviceId] = [];
      return [];
    }
  }

  async function saveMappedSpaces(
    peerDeviceId: string,
    mappedSpaceIds: string[],
  ): Promise<string[]> {
    try {
      const mapped = await invoke<string[]>("space_sync_update_mappings", {
        peerDeviceId,
        mappedSpaceIds,
      });
      mappedSpaceIdsByPeer.value[peerDeviceId] = mapped;
      void refreshSpaceSyncStatus(peerDeviceId);
      void runSpaceSyncTick();
      return mapped;
    } catch (error) {
      console.warn("[space-sync] failed to save mappings", error);
      throw error;
    }
  }

  async function resolveCustomSpaceMapping(
    peerDeviceId: string,
    remoteSpaceId: string,
    resolutionMode: "create_new" | "use_existing",
    existingSpaceId?: string,
    remoteSpaceName?: string,
  ) {
    const unresolvedBefore = unresolvedCustomSpacesByPeer.value[peerDeviceId] ?? [];

    const result = await invoke<SpaceMappingApplyResult>(
      "space_sync_resolve_custom_space_mapping",
      {
        peerDeviceId,
        remoteSpaceId,
        remoteSpaceName,
        resolutionMode,
        existingSpaceId,
      },
    );

    mappedSpaceIdsByPeer.value[peerDeviceId] = result.mapped_space_ids;

    const fallbackRemaining = unresolvedBefore.filter((space) => space.space_id !== remoteSpaceId);
    const unresolvedFromBackend = result.unresolved_custom_spaces.filter(
      (space) => space.space_id !== remoteSpaceId,
    );

    unresolvedCustomSpacesByPeer.value[peerDeviceId] =
      unresolvedFromBackend.length > 0 ? unresolvedFromBackend : fallbackRemaining;

    void refreshSpaceSyncStatus(peerDeviceId);
    void runSpaceSyncTick();
    return result;
  }

  async function refreshSpaceSyncStatus(peerDeviceId: string): Promise<SpaceSyncStatus | null> {
    try {
      const status = await invoke<SpaceSyncStatus>("space_sync_status", {
        peerDeviceId,
      });
      syncStatusByPeer.value[peerDeviceId] = status;

      if (status.last_synced_at) {
        const current = lastSyncedAtByPeer.value[peerDeviceId];
        if (!current) {
          lastSyncedAtByPeer.value[peerDeviceId] = status.last_synced_at;
        } else {
          const currentTs = Date.parse(current);
          const nextTs = Date.parse(status.last_synced_at);
          if (Number.isNaN(currentTs) || (!Number.isNaN(nextTs) && nextTs >= currentTs)) {
            lastSyncedAtByPeer.value[peerDeviceId] = status.last_synced_at;
          }
        }
      }

      return status;
    } catch (error) {
      console.warn("[space-sync] failed to load status", error);
      syncStatusByPeer.value[peerDeviceId] = null;
      return null;
    }
  }

  async function runSpaceSyncTick() {
    const peerIds = pairedDevices.value.map((p) => p.peer_device_id);
    for (const peerId of peerIds) {
      syncingByPeer.value[peerId] = true;
    }

    try {
      const tick = await invoke<SpaceSyncTickResult>("space_sync_tick");

      for (const peer of tick.peers) {
        if (peer.transferred_objects > 0) {
          lastSyncedAtByPeer.value[peer.peer_device_id] = peer.last_transfer_at;
        }
      }

      if (tick.applied_events > 0) {
        lastAppliedSyncAt.value = tick.ticked_at;
      }

      for (const peerId of peerIds) {
        void refreshSpaceSyncStatus(peerId);
      }
    } catch (error) {
      console.warn("[space-sync] tick failed", error);
    } finally {
      for (const peerId of peerIds) {
        syncingByPeer.value[peerId] = false;
      }
    }
  }

  async function consumeSpaceMappingUpdates() {
    try {
      const updates = await invoke<IncomingSpaceMappingUpdate[]>(
        "device_connection_consume_space_mapping_updates",
      );

      for (const update of updates) {
        try {
          const result = await invoke<SpaceMappingApplyResult>("space_sync_apply_remote_mappings", {
            peerDeviceId: update.from_device_id,
            mappedSpaceIds: update.mapped_space_ids,
            customSpaces: update.custom_spaces,
          });
          mappedSpaceIdsByPeer.value[update.from_device_id] = result.mapped_space_ids;
          unresolvedCustomSpacesByPeer.value[update.from_device_id] =
            result.unresolved_custom_spaces;
          void refreshSpaceSyncStatus(update.from_device_id);
        } catch (error) {
          console.warn("[space-sync] failed to apply remote mappings", error);
        }
      }
    } catch (error) {
      console.warn("[space-sync] failed to consume incoming mapping updates", error);
    }
  }

  function setDiscovered(items: DiscoveredDevice[]) {
    const deduped = new Map<string, DiscoveredDevice>();

    for (const item of items) {
      if (item.device_id === identity.value.device_id) continue;
      if (pairedDeviceIds.value.has(item.device_id)) continue;

      const existing = deduped.get(item.device_id);
      if (!existing || Date.parse(item.last_seen_at) > Date.parse(existing.last_seen_at)) {
        deduped.set(item.device_id, item);
      }
    }

    discoveredDevices.value = [...deduped.values()].sort((a, b) => {
      return Date.parse(b.last_seen_at) - Date.parse(a.last_seen_at);
    });
  }

  function pruneRequests() {
    const now = Date.now();
    const retained = incomingRequests.value.filter((req) => {
      const expiresAt = Date.parse(req.expires_at);
      if (Number.isNaN(expiresAt)) return true;
      return expiresAt > now;
    });
    const retainedIds = new Set(retained.map((item) => item.request_id));

    incomingRequests.value = retained;
    for (const requestId of Object.keys(incomingExpectedCode.value)) {
      if (!retainedIds.has(requestId)) {
        delete incomingExpectedCode.value[requestId];
        delete incomingAttemptCount.value[requestId];
        delete incomingCooldownUntil.value[requestId];
      }
    }

    if (outgoingRequest.value) {
      const expiresAt = Date.parse(outgoingRequest.value.expires_at);
      if (!Number.isNaN(expiresAt) && expiresAt <= now && outgoingRequest.value.status === "pending") {
        outgoingRequest.value = {
          ...outgoingRequest.value,
          status: "expired",
        };
      }
    }
  }

  async function refreshIdentity() {
    try {
      const resolved = await invoke<LocalDeviceIdentity>("device_connection_get_identity");
      if (resolved?.device_id && resolved?.hostname) {
        identity.value = {
          device_id: resolved.device_id,
          hostname: resolved.hostname,
        };
      }
    } catch (error) {
      console.warn("[device-connection] failed to load identity", error);
    }
  }

  async function refreshDebugStatus() {
    try {
      debugStatus.value = await invoke<DeviceConnectionDebugStatus>("device_connection_debug_status");
    } catch {
      debugStatus.value = null;
    }
  }

  async function applyPresence(items: DiscoveredDevice[]) {
    const byId = new Map(items.map((item) => [item.device_id, item]));

    for (const device of pairedDevices.value) {
      const seen = byId.get(device.peer_device_id);
      if (!seen) continue;

      const nextLastSeenAt = seen.last_seen_at || device.last_seen_at;
      if (nextLastSeenAt && nextLastSeenAt !== device.last_seen_at) {
        try {
          await invoke("device_connection_update_last_seen", {
            peerDeviceId: device.peer_device_id,
            lastSeenAt: nextLastSeenAt,
          });
        } catch (error) {
          console.warn("[device-connection] failed to update last_seen", error);
        }
      }
    }

    await loadPairedDevices();
  }

  async function refreshPresence() {
    try {
      const items = await invoke<DiscoveredDevice[]>("device_connection_presence_snapshot");
      await applyPresence(items);
    } catch (error) {
      console.warn("[device-connection] presence refresh failed", error);
    }
  }

  async function refreshDiscovery() {
    addModeLastRefreshAt.value = nowIso();

    try {
      const items = await invoke<DiscoveredDevice[]>("device_connection_discovery_snapshot");
      await applyPresence(items);
      setDiscovered(items);
    } catch (error) {
      console.warn("[device-connection] discovery snapshot refresh failed", error);
    }

    try {
      const incoming = await invoke<IncomingPairRequest[]>(
        "device_connection_pair_incoming_requests",
      );
      incomingRequests.value = incoming.map((request) => {
        const attempts = incomingAttemptCount.value[request.request_id] ?? request.attempts;
        const cooldown_until = incomingCooldownUntil.value[request.request_id] ?? request.cooldown_until;
        return {
          ...request,
          attempts,
          cooldown_until,
        };
      });
    } catch (error) {
      console.warn("[device-connection] incoming requests refresh failed", error);
    }

    try {
      const outgoingUpdates = await invoke<PairCodeUpdate[]>(
        "device_connection_pair_outgoing_updates",
      );

      if (outgoingRequest.value) {
        const update = outgoingUpdates.find(
          (item) => item.request_id === outgoingRequest.value?.request_id,
        );
        if (update) {
          outgoingRequest.value = {
            ...outgoingRequest.value,
            status: "awaiting_code",
            sender_code: update.code,
          };
        }
      }
    } catch (error) {
      console.warn("[device-connection] outgoing updates refresh failed", error);
    }

    try {
      const completions = await invoke<PairCompletionUpdate[]>(
        "device_connection_pair_outgoing_completions",
      );

      if (outgoingRequest.value) {
        const completion = completions.find(
          (item) => item.request_id === outgoingRequest.value?.request_id,
        );

        if (completion) {
          await savePairedDevice(outgoingRequest.value.to_device_id, outgoingRequest.value.to_hostname);
          outgoingRequest.value = null;
          pairCompletedAt.value = nowIso();
        }
      }
    } catch (error) {
      console.warn("[device-connection] outgoing completions refresh failed", error);
    }

    pruneRequests();
    void refreshDebugStatus();
  }

  function startDiscoveryLoop() {
    if (discoveryTimer) return;
    void refreshDiscovery();
    discoveryTimer = setInterval(() => {
      void refreshDiscovery();
    }, ADD_MODE_POLL_INTERVAL_MS);
  }

  function stopDiscoveryLoop() {
    if (!discoveryTimer) return;
    clearInterval(discoveryTimer);
    discoveryTimer = null;
  }

  function startPresenceLoop() {
    if (presenceTimer) return;
    void refreshPresence();
    presenceTimer = setInterval(() => {
      void refreshPresence();
    }, PRESENCE_POLL_INTERVAL_MS);
  }

  function startMappingUpdateLoop() {
    if (mappingUpdateTimer) return;
    void consumeSpaceMappingUpdates();
    void runSpaceSyncTick();
    mappingUpdateTimer = setInterval(() => {
      void consumeSpaceMappingUpdates();
      void runSpaceSyncTick();
    }, MAPPING_UPDATE_POLL_INTERVAL_MS);
  }

  async function hydrate() {
    await loadPairedDevices();
    await refreshIdentity();
    setDiscovered([]);
    startPresenceLoop();
    startMappingUpdateLoop();
    void refreshPresence();
    void refreshDebugStatus();
  }

  async function enterAddMode() {
    addModeEnabled.value = true;
    pairCompletedAt.value = null;
    await refreshIdentity();

    try {
      await invoke("device_connection_enter_add_mode");
    } catch (error) {
      console.warn("[device-connection] enter add mode failed", error);
    }

    startDiscoveryLoop();
  }

  async function leaveAddMode() {
    addModeEnabled.value = false;
    pairCompletedAt.value = null;
    stopDiscoveryLoop();

    try {
      await invoke("device_connection_leave_add_mode");
    } catch (error) {
      console.warn("[device-connection] leave add mode failed", error);
    }

    incomingRequests.value = [];
    incomingExpectedCode.value = {};
    incomingAttemptCount.value = {};
    incomingCooldownUntil.value = {};
    outgoingRequest.value = null;
    discoveredDevices.value = [];
    addModeLastRefreshAt.value = null;
    void refreshDebugStatus();
  }

  async function requestPair(device: DiscoveredDevice) {
    const existingRequest = outgoingRequest.value;
    if (existingRequest && (existingRequest.status === "pending" || existingRequest.status === "awaiting_code")) {
      return;
    }

    const requestId = randomId();
    const payload: DevicePairRequestInput = {
      request_id: requestId,
      to_device_id: device.device_id,
      to_addr: device.addr,
    };

    outgoingRequest.value = {
      request_id: requestId,
      to_device_id: device.device_id,
      to_hostname: device.hostname,
      created_at: nowIso(),
      expires_at: new Date(Date.now() + REQUEST_TTL_MS).toISOString(),
      status: "pending",
      sender_code: null,
    };

    try {
      await invoke("device_connection_send_pair_request", { input: payload });
      void refreshDiscovery();
    } catch (error) {
      console.warn("[device-connection] pair request send failed", error);
      if (outgoingRequest.value) {
        outgoingRequest.value = {
          ...outgoingRequest.value,
          status: "rejected",
        };
      }
    }
  }

  function cancelOutgoingRequest() {
    outgoingRequest.value = null;
  }

  async function acceptIncomingRequest(requestId: string) {
    const payload: DevicePairRequestAckInput = { request_id: requestId };
    try {
      const update = await invoke<PairCodeUpdate>(
        "device_connection_pair_accept_request",
        { input: payload },
      );
      incomingExpectedCode.value[requestId] = update.code;
      incomingAttemptCount.value[requestId] = 0;
      incomingCooldownUntil.value[requestId] = null;
      void refreshDiscovery();
      return true;
    } catch (error) {
      console.warn("[device-connection] accept request failed", error);
      return false;
    }
  }

  async function rejectIncomingRequest(requestId: string) {
    const payload: DevicePairRequestAckInput = { request_id: requestId };
    try {
      await invoke("device_connection_pair_acknowledge_request", { input: payload });
    } catch (error) {
      console.warn("[device-connection] reject request ack failed", error);
    }
    delete incomingExpectedCode.value[requestId];
    delete incomingAttemptCount.value[requestId];
    delete incomingCooldownUntil.value[requestId];
    incomingRequests.value = incomingRequests.value.filter((item) => item.request_id !== requestId);
    void refreshDiscovery();
  }

  async function submitPairCode(requestId: string, code: string): Promise<boolean> {
    const request = incomingRequests.value.find((item) => item.request_id === requestId);
    if (!request) return false;

    const normalized = code.replace(/\D/g, "").slice(0, 6);
    if (normalized.length !== 6) return false;

    const cooldownUntil = incomingCooldownUntil.value[requestId] ?? request.cooldown_until;
    if (cooldownUntil && Date.parse(cooldownUntil) > Date.now()) {
      return false;
    }

    const expectedCode = incomingExpectedCode.value[requestId];
    if (!expectedCode) return false;

    if (normalized !== expectedCode) {
      const nextAttempts = (incomingAttemptCount.value[requestId] ?? request.attempts ?? 0) + 1;
      incomingAttemptCount.value[requestId] = nextAttempts;

      if (nextAttempts >= 3) {
        const cooldown = new Date(Date.now() + 60_000).toISOString();
        incomingAttemptCount.value[requestId] = 0;
        incomingCooldownUntil.value[requestId] = cooldown;
      }

      incomingRequests.value = incomingRequests.value.map((item) => {
        if (item.request_id !== requestId) return item;
        return {
          ...item,
          attempts: incomingAttemptCount.value[requestId] ?? item.attempts,
          cooldown_until: incomingCooldownUntil.value[requestId] ?? item.cooldown_until,
        };
      });

      return false;
    }

    await savePairedDevice(request.from_device_id, request.from_hostname);

    try {
      await invoke("device_connection_pair_complete_request", {
        input: { request_id: requestId } satisfies DevicePairRequestAckInput,
      });
    } catch (error) {
      console.warn("[device-connection] submit code completion failed", error);
    }

    delete incomingExpectedCode.value[requestId];
    delete incomingAttemptCount.value[requestId];
    delete incomingCooldownUntil.value[requestId];
    incomingRequests.value = incomingRequests.value.filter((item) => item.request_id !== requestId);
    pairCompletedAt.value = nowIso();
    void refreshDiscovery();
    return true;
  }

  async function unpairDevice(deviceId: string) {
    try {
      await invoke("device_connection_unpair", { peerDeviceId: deviceId });
    } catch (error) {
      console.warn("[device-connection] unpair failed", error);
    }
    delete mappedSpaceIdsByPeer.value[deviceId];
    delete unresolvedCustomSpacesByPeer.value[deviceId];
    delete syncStatusByPeer.value[deviceId];
    delete lastSyncedAtByPeer.value[deviceId];
    delete syncingByPeer.value[deviceId];
    await loadPairedDevices();
  }

  function isDeviceOnline(device: PairedDevice): boolean {
    if (!device.last_seen_at) return false;
    const lastSeen = Date.parse(device.last_seen_at);
    if (Number.isNaN(lastSeen)) return false;
    return Date.now() - lastSeen <= OFFLINE_AFTER_MISSED_HEARTBEATS_MS;
  }

  function shortDeviceId(deviceId: string): string {
    return shortUuid(deviceId, 6, "end");
  }

  function findPairedDevice(deviceId: string): PairedDevice | null {
    return pairedDevices.value.find((device) => device.peer_device_id === deviceId) ?? null;
  }

  return {
    identity,
    pairedDevices,
    discoveredDevices,
    incomingRequests,
    outgoingRequest,
    outgoingRequestSecondsLeft,
    addModeEnabled,
    addModeLastRefreshAt,
    debugStatus,
    pairCompletedAt,
    mappedSpaceIdsByPeer,
    unresolvedCustomSpacesByPeer,
    syncStatusByPeer,
    lastSyncedAtByPeer,
    syncingByPeer,
    lastAppliedSyncAt,
    hydrate,
    refreshDiscovery,
    refreshDebugStatus,
    loadMappedSpaces,
    saveMappedSpaces,
    resolveCustomSpaceMapping,
    getMappedSpaceIds,
    getUnresolvedCustomSpaces,
    refreshSpaceSyncStatus,
    getSpaceSyncStatus,
    getLastSyncedAt,
    isSyncingPeer,
    runSpaceSyncTick,
    enterAddMode,
    leaveAddMode,
    requestPair,
    cancelOutgoingRequest,
    acceptIncomingRequest,
    rejectIncomingRequest,
    submitPairCode,
    unpairDevice,
    isDeviceOnline,
    shortDeviceId,
    findPairedDevice,
  };
});
