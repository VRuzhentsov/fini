import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import { computed, ref } from "vue";

export interface PairedDevice {
  id: string;
  display_name: string;
  paired_at: string;
  last_seen_at: string | null;
  pair_state: "paired";
}

export interface DiscoveredDevice {
  device_id: string;
  hostname: string;
  addr: string;
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

export interface DeviceSyncDebugStatus {
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

interface DevicePairRequestInput {
  request_id: string;
  to_device_id: string;
  to_addr: string;
}

interface DevicePairRequestAckInput {
  request_id: string;
}

interface StoredDeviceRuntime {
  paired_devices: PairedDevice[];
}

interface LocalDeviceIdentity {
  device_id: string;
  hostname: string;
}

const RUNTIME_KEY = "fini.device_sync.runtime";
export const ADD_MODE_DISCOVERY_INTERVAL_MS = 5_000;
const ADD_MODE_POLL_INTERVAL_MS = 1_000;
const PRESENCE_POLL_INTERVAL_MS = 15_000;
const NORMAL_HEARTBEAT_MS = 60_000;
const OFFLINE_AFTER_MISSED_HEARTBEATS_MS = NORMAL_HEARTBEAT_MS * 2;
const REQUEST_TTL_MS = 60_000;

function parseJson<T>(raw: string | null): T | null {
  if (!raw) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

function nowIso(): string {
  return new Date().toISOString();
}

function randomId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `${Date.now().toString(16)}-${Math.random().toString(16).slice(2, 10)}`;
}

function toRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null) return null;
  return value as Record<string, unknown>;
}

function loadRuntime(): StoredDeviceRuntime {
  if (typeof window === "undefined") {
    return { paired_devices: [] };
  }

  const parsed = parseJson<StoredDeviceRuntime>(window.localStorage.getItem(RUNTIME_KEY));
  if (!parsed || !Array.isArray(parsed.paired_devices)) {
    return { paired_devices: [] };
  }

  const paired_devices: PairedDevice[] = parsed.paired_devices
    .map((raw) => {
      const record = toRecord(raw);
      if (!record) return null;
      if (typeof record.id !== "string") return null;
      if (typeof record.display_name !== "string") return null;
      if (typeof record.paired_at !== "string") return null;
      const last_seen_at = typeof record.last_seen_at === "string" ? record.last_seen_at : null;

      return {
        id: record.id,
        display_name: record.display_name,
        paired_at: record.paired_at,
        last_seen_at,
        pair_state: "paired",
      } satisfies PairedDevice;
    })
    .filter((device): device is PairedDevice => device !== null);

  return { paired_devices };
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
  const debugStatus = ref<DeviceSyncDebugStatus | null>(null);
  const pairCompletedAt = ref<string | null>(null);
  const incomingExpectedCode = ref<Record<string, string>>({});
  const incomingAttemptCount = ref<Record<string, number>>({});
  const incomingCooldownUntil = ref<Record<string, string | null>>({});

  let discoveryTimer: ReturnType<typeof setInterval> | null = null;
  let presenceTimer: ReturnType<typeof setInterval> | null = null;

  const outgoingRequestSecondsLeft = computed(() => {
    if (!outgoingRequest.value) return 0;
    const expiresAt = Date.parse(outgoingRequest.value.expires_at);
    const diff = expiresAt - Date.now();
    return diff > 0 ? Math.ceil(diff / 1000) : 0;
  });

  const pairedDeviceIds = computed(() => new Set(pairedDevices.value.map((device) => device.id)));

  function persistRuntime() {
    if (typeof window === "undefined") return;
    const payload: StoredDeviceRuntime = {
      paired_devices: pairedDevices.value,
    };
    window.localStorage.setItem(RUNTIME_KEY, JSON.stringify(payload));
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
      const resolved = await invoke<LocalDeviceIdentity>("device_get_identity");
      if (resolved?.device_id && resolved?.hostname) {
        identity.value = {
          device_id: resolved.device_id,
          hostname: resolved.hostname,
        };
      }
    } catch (error) {
      console.warn("[device-sync] failed to load identity", error);
    }
  }

  async function refreshDebugStatus() {
    try {
      debugStatus.value = await invoke<DeviceSyncDebugStatus>("device_sync_debug_status");
    } catch {
      debugStatus.value = null;
    }
  }

  function applyPresence(items: DiscoveredDevice[]) {
    const byId = new Map(items.map((item) => [item.device_id, item]));
    let changed = false;

    pairedDevices.value = pairedDevices.value.map((device) => {
      const seen = byId.get(device.id);
      if (!seen) return device;

      const nextDisplayName = seen.hostname || device.display_name;
      const nextLastSeenAt = seen.last_seen_at || device.last_seen_at;

      if (nextDisplayName === device.display_name && nextLastSeenAt === device.last_seen_at) {
        return device;
      }

      changed = true;
      return {
        ...device,
        display_name: nextDisplayName,
        last_seen_at: nextLastSeenAt,
      };
    });

    if (changed) {
      persistRuntime();
    }
  }

  async function refreshPresence() {
    try {
      const items = await invoke<DiscoveredDevice[]>("device_presence_snapshot");
      applyPresence(items);
    } catch (error) {
      console.warn("[device-sync] presence refresh failed", error);
    }
  }

  async function refreshDiscovery() {
    addModeLastRefreshAt.value = nowIso();

    try {
      const items = await invoke<DiscoveredDevice[]>("device_discovery_snapshot");
      applyPresence(items);
      setDiscovered(items);
    } catch (error) {
      console.warn("[device-sync] discovery snapshot refresh failed", error);
    }

    try {
      const incoming = await invoke<IncomingPairRequest[]>("device_pair_incoming_requests");
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
      console.warn("[device-sync] incoming requests refresh failed", error);
    }

    try {
      const outgoingUpdates = await invoke<PairCodeUpdate[]>("device_pair_outgoing_updates");

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
      console.warn("[device-sync] outgoing updates refresh failed", error);
    }

    try {
      const completions = await invoke<PairCompletionUpdate[]>("device_pair_outgoing_completions");

      if (outgoingRequest.value) {
        const completion = completions.find(
          (item) => item.request_id === outgoingRequest.value?.request_id,
        );

        if (completion) {
          upsertPairedDevice(outgoingRequest.value.to_device_id, outgoingRequest.value.to_hostname);
          persistRuntime();
          outgoingRequest.value = null;
          pairCompletedAt.value = nowIso();
        }
      }
    } catch (error) {
      console.warn("[device-sync] outgoing completions refresh failed", error);
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

  async function hydrate() {
    pairedDevices.value = loadRuntime().paired_devices;
    await refreshIdentity();
    setDiscovered([]);
    startPresenceLoop();
    void refreshPresence();
    void refreshDebugStatus();
  }

  async function enterAddMode() {
    addModeEnabled.value = true;
    pairCompletedAt.value = null;
    await refreshIdentity();

    try {
      await invoke("device_enter_add_mode");
    } catch (error) {
      console.warn("[device-sync] enter add mode failed", error);
    }

    startDiscoveryLoop();
  }

  async function leaveAddMode() {
    addModeEnabled.value = false;
    pairCompletedAt.value = null;
    stopDiscoveryLoop();

    try {
      await invoke("device_leave_add_mode");
    } catch (error) {
      console.warn("[device-sync] leave add mode failed", error);
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
      await invoke("device_send_pair_request", { input: payload });
      void refreshDiscovery();
    } catch (error) {
      console.warn("[device-sync] pair request send failed", error);
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
      const update = await invoke<PairCodeUpdate>("device_pair_accept_request", { input: payload });
      incomingExpectedCode.value[requestId] = update.code;
      incomingAttemptCount.value[requestId] = 0;
      incomingCooldownUntil.value[requestId] = null;
      void refreshDiscovery();
      return true;
    } catch (error) {
      console.warn("[device-sync] accept request failed", error);
      return false;
    }
  }

  async function rejectIncomingRequest(requestId: string) {
    const payload: DevicePairRequestAckInput = { request_id: requestId };
    try {
      await invoke("device_pair_acknowledge_request", { input: payload });
    } catch (error) {
      console.warn("[device-sync] reject request ack failed", error);
    }
    delete incomingExpectedCode.value[requestId];
    delete incomingAttemptCount.value[requestId];
    delete incomingCooldownUntil.value[requestId];
    incomingRequests.value = incomingRequests.value.filter((item) => item.request_id !== requestId);
    void refreshDiscovery();
  }

  function upsertPairedDevice(deviceId: string, displayName: string) {
    const pairedAt = nowIso();
    const existing = pairedDevices.value.find((item) => item.id === deviceId);
    if (existing) {
      existing.display_name = displayName;
      existing.last_seen_at = pairedAt;
      return;
    }

    pairedDevices.value.unshift({
      id: deviceId,
      display_name: displayName,
      paired_at: pairedAt,
      last_seen_at: pairedAt,
      pair_state: "paired",
    });
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

    upsertPairedDevice(request.from_device_id, request.from_hostname);
    persistRuntime();

    try {
      await invoke("device_pair_complete_request", {
        input: { request_id: requestId } satisfies DevicePairRequestAckInput,
      });
    } catch (error) {
      console.warn("[device-sync] submit code completion failed", error);
    }

    delete incomingExpectedCode.value[requestId];
    delete incomingAttemptCount.value[requestId];
    delete incomingCooldownUntil.value[requestId];
    incomingRequests.value = incomingRequests.value.filter((item) => item.request_id !== requestId);
    pairCompletedAt.value = nowIso();
    void refreshDiscovery();
    return true;
  }

  function unpairDevice(deviceId: string) {
    pairedDevices.value = pairedDevices.value.filter((item) => item.id !== deviceId);
    persistRuntime();
  }

  function isDeviceOnline(device: PairedDevice): boolean {
    if (!device.last_seen_at) return false;
    const lastSeen = Date.parse(device.last_seen_at);
    if (Number.isNaN(lastSeen)) return false;
    return Date.now() - lastSeen <= OFFLINE_AFTER_MISSED_HEARTBEATS_MS;
  }

  function shortDeviceId(deviceId: string): string {
    if (deviceId.length <= 6) return deviceId;
    return deviceId.slice(-6);
  }

  function findPairedDevice(deviceId: string): PairedDevice | null {
    return pairedDevices.value.find((device) => device.id === deviceId) ?? null;
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
    hydrate,
    refreshDiscovery,
    refreshDebugStatus,
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
