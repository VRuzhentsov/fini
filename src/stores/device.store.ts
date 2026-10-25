import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { shortUuid } from "../utils/shortUuid";
import { listen } from "@tauri-apps/api/event";
import { ChannelColor, ChannelKind, ChannelProblem, ChannelState } from "../utils/channel";

// The store of DeviceService (ADR-0010 D16): this device, the paired
// devices, Add Device and pairing, and the channels of each pair. The
// backend tells it what changed through `fini-event`; it reads the state
// again then, with no polling.

export { ChannelColor, ChannelKind, ChannelProblem, ChannelState };

export interface PairedDevice {
  peer_device_id: string;
  display_name: string;
  paired_at: string;
  last_seen_at: string | null;
  pair_state: string;
}

export interface DeviceChannelStatus {
  kind: ChannelKind;
  state: ChannelState;
  color: ChannelColor;
  // Set only on an orange row.
  problem: ChannelProblem | null;
  // The person's chosen channel for this pair's traffic (ADR-0007).
  primary: boolean;
}

// Where a channel's init stands during a setup (ADR-0008 D1).
export interface ChannelSetupStatus {
  sentRequestAccepted: boolean;
  acceptedPeerRequest: boolean;
  // Both halves done: OK can be pressed (D4).
  initialized: boolean;
}

export interface DiscoveredDevice {
  device_id: string;
  hostname: string;
  // An IP address for Network, a Bluetooth address for Bluetooth.
  addr: string;
  discovery_port: number;
  ws_port: number | null;
  last_seen_at: string;
  // The channel that heard the device.
  channel_kind: ChannelKind;
}

export interface IncomingPairRequest {
  request_id: string;
  from_device_id: string;
  from_hostname: string;
  created_at: string;
  expires_at: string;
  attempts: number;
  cooldown_until: string | null;
  via_bluetooth: boolean;
  from_bluetooth_address: string | null;
}

export interface OutgoingPairRequest {
  request_id: string;
  to_device_id: string;
  to_hostname: string;
  created_at: string;
  expires_at: string;
  // "rejected": the peer answered no. "send_failed": this side never got
  // the request out.
  status: "pending" | "awaiting_code" | "expired" | "paired" | "rejected" | "send_failed";
  sender_code: string | null;
}

export interface DeviceConnectionDebugStatus {
  add_mode_enabled: boolean;
  worker_started: boolean;
  tx_count: number;
  rx_count: number;
  discovered_count: number;
  peer_session_count: number;
  incoming_request_count: number;
  incoming_space_mapping_update_count: number;
  outgoing_code_count: number;
  last_broadcast_at: string | null;
  last_error: string | null;
  discovery_port: number;
  discovery_provider: string;
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
  via_bluetooth: boolean;
  bluetooth_address: string | null;
}

interface DevicePairRequestInput {
  request_id: string;
  to_device_id: string;
  to_addr: string;
  to_ws_port?: number | null;
}

interface DevicePairRequestBluetoothInput {
  request_id: string;
  to_device_id: string;
  to_bluetooth_address: string;
}

interface DevicePairRequestAckInput {
  request_id: string;
}

interface LocalDeviceIdentity {
  device_id: string;
  hostname: string;
}

// A Bluetooth list row that no identify step answered yet has this id
// prefix; Pair identifies the device first to learn its real id.
export const BLUETOOTH_LIST_ID_PREFIX = "bt:";
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

function setupKey(peerDeviceId: string, kind: ChannelKind): string {
  return `${peerDeviceId}:${kind}`;
}


// The events of the backend's event bus (ADR-0010 D8, D9), as the Tauri
// event `fini-event`. Each event tells that one thing changed; a store
// reads its state again when it gets one.

export const FINI_EVENT = "fini-event";

export const FiniEventType = {
  SessionChanged: "session-changed",
  ChannelsChanged: "channels-changed",
  ChannelSetupChanged: "channel-setup-changed",
  PairedDevicesChanged: "paired-devices-changed",
  AvailableDevicesChanged: "available-devices-changed",
  PairRequestsChanged: "pair-requests-changed",
  SpaceDataChanged: "space-data-changed",
  SpaceMappingRequestsChanged: "space-mapping-requests-changed",
  SyncQueueChanged: "sync-queue-changed",
  DeviceHeard: "device-heard",
  ConnectChanged: "connect-changed",
  ChannelMissingAtPeer: "channel-missing-at-peer",
} as const;

export type FiniEventType = (typeof FiniEventType)[keyof typeof FiniEventType];

export interface FiniEvent {
  type: FiniEventType;
  peerDeviceId?: string;
  kind?: string;
  established?: boolean;
}

type Subscriber = (event: FiniEvent) => void;

const subscribers: Subscriber[] = [];
let listening: Promise<unknown> | null = null;

// Calls `subscriber` with every event. One Tauri listener serves all the
// subscribers.
export async function onFiniEvent(subscriber: Subscriber): Promise<void> {
  subscribers.push(subscriber);
  listening ??= listen<FiniEvent>(FINI_EVENT, ({ payload }) => {
    for (const each of subscribers) each(payload);
  });
  await listening;
}

export const useDeviceStore = defineStore("device", () => {
  const identity = ref<LocalDeviceIdentity>({ device_id: randomId(), hostname: "fini-device" });
  const pairedDevices = ref<PairedDevice[]>([]);
  // The Add Device list, from both channels.
  const availableDevices = ref<DiscoveredDevice[]>([]);
  const incomingRequests = ref<IncomingPairRequest[]>([]);
  const outgoingRequest = ref<OutgoingPairRequest | null>(null);
  const addModeEnabled = ref(false);
  const debugStatus = ref<DeviceConnectionDebugStatus | null>(null);
  const pairCompletedAt = ref<string | null>(null);
  const channelStatusesByPeer = ref<Record<string, DeviceChannelStatus[]>>({});
  const channelSetupByKey = ref<Record<string, ChannelSetupStatus | null>>({});
  const incomingExpectedCode = ref<Record<string, string>>({});
  const incomingAttemptCount = ref<Record<string, number>>({});
  const incomingCooldownUntil = ref<Record<string, string | null>>({});

  // Lets an `enterAddMode` still waiting tell whether `leaveAddMode` ran
  // meanwhile, so that it does not open Add Device for a dialog that closed.
  let addModeGeneration = 0;
  let requestExpiryTimer: ReturnType<typeof setTimeout> | null = null;

  const outgoingRequestSecondsLeft = computed(() => {
    if (!outgoingRequest.value) return 0;
    const diff = Date.parse(outgoingRequest.value.expires_at) - Date.now();
    return diff > 0 ? Math.ceil(diff / 1000) : 0;
  });

  const pairedDeviceIds = computed(() => new Set(pairedDevices.value.map((d) => d.peer_device_id)));

  const otherDevices = computed(() =>
    availableDevices.value.filter((device) => device.device_id !== identity.value.device_id),
  );

  // The Add Device list for each channel, paired devices included: the list
  // offers "Pair again" for a pair that the other device removed.
  const discoveredWithPairedByChannel = computed<Record<ChannelKind, DiscoveredDevice[]>>(() => ({
    [ChannelKind.Network]: otherDevices.value.filter((device) => device.channel_kind === ChannelKind.Network),
    [ChannelKind.Bluetooth]: otherDevices.value.filter((device) => device.channel_kind === ChannelKind.Bluetooth),
  }));

  // The same, without the devices paired here.
  const discoveredByChannel = computed<Record<ChannelKind, DiscoveredDevice[]>>(() => ({
    [ChannelKind.Network]: discoveredWithPairedByChannel.value[ChannelKind.Network].filter(
      (device) => !pairedDeviceIds.value.has(device.device_id),
    ),
    [ChannelKind.Bluetooth]: discoveredWithPairedByChannel.value[ChannelKind.Bluetooth].filter(
      (device) => !pairedDeviceIds.value.has(device.device_id),
    ),
  }));

  // One list over both channels: Network wins for a device both heard.
  const discoveredDevices = computed<DiscoveredDevice[]>(() => {
    const byId = new Map<string, DiscoveredDevice>();
    for (const kind of [ChannelKind.Network, ChannelKind.Bluetooth]) {
      for (const device of discoveredByChannel.value[kind]) {
        if (!byId.has(device.device_id)) byId.set(device.device_id, device);
      }
    }
    return [...byId.values()].sort((a, b) => Date.parse(b.last_seen_at) - Date.parse(a.last_seen_at));
  });

  async function loadPairedDevices() {
    try {
      pairedDevices.value = await invoke<PairedDevice[]>("device_paired_devices");
    } catch (error) {
      console.warn("[device] failed to load paired devices", error);
    }
  }

  // Lets its errors through: both callers sent PairComplete to the peer
  // before this write, so a failure here leaves the two devices different,
  // and the caller must not report success. `requestId` names the pairing:
  // the backend pins the key that came with it.
  async function savePairedDevice(
    deviceId: string,
    displayName: string,
    requestId: string,
    bluetoothAddress: string | null = null,
    viaBluetooth = false,
  ) {
    await invoke<PairedDevice>("device_save_paired_device", {
      peerDeviceId: deviceId,
      displayName,
      bluetoothAddress,
      viaBluetooth,
      requestId,
    });
    await loadPairedDevices();
  }

  function getChannelStatuses(peerDeviceId: string): DeviceChannelStatus[] {
    return channelStatusesByPeer.value[peerDeviceId] ?? [];
  }

  async function refreshChannelStatuses(peerDeviceId: string): Promise<DeviceChannelStatus[]> {
    try {
      const statuses = await invoke<DeviceChannelStatus[]>("device_channel_statuses", { peerDeviceId });
      channelStatusesByPeer.value[peerDeviceId] = statuses;
      return statuses;
    } catch (error) {
      console.warn("[device] failed to load channel statuses", error);
      channelStatusesByPeer.value[peerDeviceId] = [];
      return [];
    }
  }

  // The switch on a channel row, for either kind. Returns the pair's rows
  // as they are after the write.
  async function setChannelEnabled(peerDeviceId: string, kind: ChannelKind, enabled: boolean): Promise<DeviceChannelStatus[]> {
    const statuses = await invoke<DeviceChannelStatus[]>("device_set_channel_enabled", { peerDeviceId, kind, enabled });
    channelStatusesByPeer.value[peerDeviceId] = statuses;
    return statuses;
  }

  // The star on a channel row (ADR-0007). `null` clears the choice.
  async function setPrimaryChannel(peerDeviceId: string, kind: ChannelKind | null): Promise<DeviceChannelStatus[]> {
    const statuses = await invoke<DeviceChannelStatus[]>("device_set_primary_channel", { peerDeviceId, primary: kind });
    channelStatusesByPeer.value[peerDeviceId] = statuses;
    return statuses;
  }

  // "Unlink" on the Device page. The backend rejects it while the channel
  // is on.
  async function unlinkChannel(peerDeviceId: string, kind: ChannelKind): Promise<DeviceChannelStatus[]> {
    const statuses = await invoke<DeviceChannelStatus[]>("device_unlink_channel", { peerDeviceId, kind });
    channelStatusesByPeer.value[peerDeviceId] = statuses;
    return statuses;
  }

  // ADR-0008 D2: starts the setup of a paired device's channel. Rejects
  // when Bluetooth does not work on this device (D6).
  async function beginChannelSetup(peerDeviceId: string, kind: ChannelKind): Promise<void> {
    await invoke("device_begin_channel_setup", { peerDeviceId, kind });
    await refreshChannelSetup(peerDeviceId, kind);
  }

  // How far the channel's init got; null when no setup is open.
  function getChannelSetup(peerDeviceId: string, kind: ChannelKind): ChannelSetupStatus | null {
    return channelSetupByKey.value[setupKey(peerDeviceId, kind)] ?? null;
  }

  async function refreshChannelSetup(peerDeviceId: string, kind: ChannelKind): Promise<ChannelSetupStatus | null> {
    const status = await invoke<ChannelSetupStatus | null>("device_channel_setup_status", { peerDeviceId, kind });
    channelSetupByKey.value = { ...channelSetupByKey.value, [setupKey(peerDeviceId, kind)]: status };
    return status;
  }

  // OK (`switchOn`) or the dialog closed. The backend writes the channel
  // only if its init completed (ADR-0008 D15).
  async function endChannelSetup(peerDeviceId: string, kind: ChannelKind, switchOn: boolean): Promise<DeviceChannelStatus[]> {
    const statuses = await invoke<DeviceChannelStatus[]>("device_end_channel_setup", { peerDeviceId, kind, switchOn });
    channelStatusesByPeer.value[peerDeviceId] = statuses;
    return statuses;
  }

  // Removes the pair requests that expired, and marks this device's own
  // request expired.
  function pruneRequests() {
    const now = Date.now();
    const retained = incomingRequests.value.filter((req) => {
      const expiresAt = Date.parse(req.expires_at);
      return Number.isNaN(expiresAt) || expiresAt > now;
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
        outgoingRequest.value = { ...outgoingRequest.value, status: "expired" };
      }
    }
  }

  // Prunes again when the next request expires: a time, not a change the
  // backend tells.
  function schedulePruneAtNextExpiry() {
    if (requestExpiryTimer) clearTimeout(requestExpiryTimer);
    requestExpiryTimer = null;
    const times = [
      ...incomingRequests.value.map((req) => Date.parse(req.expires_at)),
      outgoingRequest.value ? Date.parse(outgoingRequest.value.expires_at) : Number.NaN,
    ].filter((time) => !Number.isNaN(time) && time > Date.now());
    if (times.length === 0) return;
    requestExpiryTimer = setTimeout(() => {
      pruneRequests();
      schedulePruneAtNextExpiry();
    }, Math.min(...times) - Date.now() + 50);
  }

  async function refreshIdentity() {
    try {
      const resolved = await invoke<LocalDeviceIdentity>("device_get_identity");
      if (resolved?.device_id && resolved?.hostname) {
        identity.value = { device_id: resolved.device_id, hostname: resolved.hostname };
      }
    } catch (error) {
      console.warn("[device] failed to load identity", error);
    }
  }

  async function refreshDebugStatus() {
    try {
      debugStatus.value = await invoke<DeviceConnectionDebugStatus>("device_debug_status");
    } catch {
      debugStatus.value = null;
    }
  }

  async function refreshAvailableDevices() {
    if (!addModeEnabled.value) {
      availableDevices.value = [];
      return;
    }
    try {
      availableDevices.value = await invoke<DiscoveredDevice[]>("device_available_devices");
    } catch (error) {
      console.warn("[device] reading the Add Device list failed", error);
    }
  }

  // The pair requests, and the answers to this device's own request.
  async function refreshPairing() {
    try {
      const incoming = await invoke<IncomingPairRequest[]>("device_pair_incoming_requests");
      incomingRequests.value = incoming.map((request) => ({
        ...request,
        attempts: incomingAttemptCount.value[request.request_id] ?? request.attempts,
        cooldown_until: incomingCooldownUntil.value[request.request_id] ?? request.cooldown_until,
      }));
    } catch (error) {
      console.warn("[device] reading the incoming requests failed", error);
    }

    if (outgoingRequest.value) {
      try {
        const updates = await invoke<PairCodeUpdate[]>("device_pair_outgoing_updates");
        const update = updates.find((item) => item.request_id === outgoingRequest.value?.request_id);
        if (update && outgoingRequest.value) {
          outgoingRequest.value = { ...outgoingRequest.value, status: "awaiting_code", sender_code: update.code };
        }
      } catch (error) {
        console.warn("[device] reading the outgoing updates failed", error);
      }

      try {
        const completions = await invoke<PairCompletionUpdate[]>("device_pair_outgoing_completions");
        const completion = completions.find((item) => item.request_id === outgoingRequest.value?.request_id);
        if (completion && outgoingRequest.value) {
          await savePairedDevice(
            outgoingRequest.value.to_device_id,
            outgoingRequest.value.to_hostname,
            completion.request_id,
            completion.bluetooth_address,
            completion.via_bluetooth,
          );
          outgoingRequest.value = null;
          pairCompletedAt.value = nowIso();
        }
      } catch (error) {
        console.warn("[device] reading the outgoing completions failed", error);
      }
    }

    pruneRequests();
    schedulePruneAtNextExpiry();
  }

  async function refreshAllChannelStatuses() {
    await Promise.all(pairedDevices.value.map((device) => refreshChannelStatuses(device.peer_device_id)));
  }

  let started = false;

  // The first read of each screen's state, then the events of the backend.
  async function hydrate() {
    if (started) return;
    started = true;
    await onFiniEvent((event) => {
      switch (event.type) {
        case FiniEventType.SessionChanged:
        case FiniEventType.ChannelsChanged:
          if (event.peerDeviceId) void refreshChannelStatuses(event.peerDeviceId);
          break;
        case FiniEventType.ChannelSetupChanged:
          if (event.peerDeviceId && event.kind) void refreshChannelSetup(event.peerDeviceId, event.kind as ChannelKind);
          break;
        case FiniEventType.PairedDevicesChanged:
          void loadPairedDevices().then(refreshAllChannelStatuses);
          break;
        case FiniEventType.AvailableDevicesChanged:
          void refreshAvailableDevices();
          break;
        case FiniEventType.PairRequestsChanged:
          void refreshPairing();
          break;
      }
    });
    await loadPairedDevices();
    await refreshIdentity();
    await refreshAllChannelStatuses();
    void refreshDebugStatus();
  }

  // `bluetooth: false` when only the Network channel is being added: no
  // Bluetooth advertising, permission prompt, or scan for it.
  async function enterAddMode({ bluetooth = true }: { bluetooth?: boolean } = {}) {
    const generation = ++addModeGeneration;
    addModeEnabled.value = true;
    pairCompletedAt.value = null;
    await refreshIdentity();
    if (generation !== addModeGeneration) return;
    try {
      await invoke("device_enter_add_mode", { bluetooth });
    } catch (error) {
      console.warn("[device] enter add mode failed", error);
    }
    if (generation !== addModeGeneration) {
      // A `leaveAddMode` ran meanwhile: leave again, unless a newer
      // `enterAddMode` owns Add Device now.
      if (!addModeEnabled.value) {
        try {
          await invoke("device_leave_add_mode");
        } catch (error) {
          console.warn("[device] leave add mode (late) failed", error);
        }
      }
      return;
    }
    await refreshAvailableDevices();
    await refreshPairing();
  }

  async function leaveAddMode() {
    addModeGeneration += 1;
    const generation = addModeGeneration;
    addModeEnabled.value = false;
    pairCompletedAt.value = null;
    try {
      await invoke("device_leave_add_mode");
    } catch (error) {
      console.warn("[device] leave add mode failed", error);
    }
    // A newer `enterAddMode` took over while the leave was in flight: its
    // state is the one that holds.
    if (generation !== addModeGeneration) return;
    incomingRequests.value = [];
    incomingExpectedCode.value = {};
    incomingAttemptCount.value = {};
    incomingCooldownUntil.value = {};
    outgoingRequest.value = null;
    availableDevices.value = [];
    void refreshDebugStatus();
  }

  async function requestPair(listed: DiscoveredDevice) {
    const existingRequest = outgoingRequest.value;
    if (existingRequest && (existingRequest.status === "pending" || existingRequest.status === "awaiting_code")) {
      return;
    }
    const requestId = randomId();
    outgoingRequest.value = {
      request_id: requestId,
      to_device_id: listed.device_id,
      to_hostname: listed.hostname,
      created_at: nowIso(),
      expires_at: new Date(Date.now() + REQUEST_TTL_MS).toISOString(),
      status: "pending",
      sender_code: null,
    };
    try {
      // A Bluetooth row from an advertisement alone, or a device paired here
      // that is paired again: one connection to learn its id, name and key.
      let device = listed;
      const needsIdentify =
        listed.device_id.startsWith(BLUETOOTH_LIST_ID_PREFIX) || pairedDeviceIds.value.has(listed.device_id);
      if (listed.channel_kind === ChannelKind.Bluetooth && needsIdentify) {
        device = await invoke<DiscoveredDevice>("device_identify_bluetooth_device", { deviceId: listed.device_id });
        if (outgoingRequest.value?.request_id === requestId) {
          outgoingRequest.value = { ...outgoingRequest.value, to_device_id: device.device_id, to_hostname: device.hostname };
        }
      }
      if (device.channel_kind === ChannelKind.Bluetooth) {
        const input: DevicePairRequestBluetoothInput = {
          request_id: requestId,
          to_device_id: device.device_id,
          to_bluetooth_address: device.addr,
        };
        await invoke("device_send_pair_request_bluetooth", { input });
      } else {
        const input: DevicePairRequestInput = {
          request_id: requestId,
          to_device_id: device.device_id,
          to_addr: device.addr,
          to_ws_port: device.ws_port,
        };
        await invoke("device_send_pair_request", { input });
      }
      // The peer's request lives from when it arrived, and a Bluetooth send
      // can take seconds: count the expiry from now.
      if (outgoingRequest.value?.request_id === requestId) {
        outgoingRequest.value = {
          ...outgoingRequest.value,
          expires_at: new Date(Date.now() + REQUEST_TTL_MS).toISOString(),
        };
      }
      void refreshPairing();
    } catch (error) {
      console.warn("[device] pair request send failed", error);
      if (outgoingRequest.value) {
        // The send failed: the peer never got the question, so this is no
        // decline.
        outgoingRequest.value = { ...outgoingRequest.value, status: "send_failed" };
      }
    }
  }

  function cancelOutgoingRequest() {
    outgoingRequest.value = null;
  }

  async function acceptIncomingRequest(requestId: string) {
    const input: DevicePairRequestAckInput = { request_id: requestId };
    try {
      const update = await invoke<PairCodeUpdate>("device_pair_accept_request", { input });
      incomingExpectedCode.value[requestId] = update.code;
      incomingAttemptCount.value[requestId] = 0;
      incomingCooldownUntil.value[requestId] = null;
      void refreshPairing();
      return true;
    } catch (error) {
      console.warn("[device] accept request failed", error);
      return false;
    }
  }

  async function rejectIncomingRequest(requestId: string) {
    const input: DevicePairRequestAckInput = { request_id: requestId };
    try {
      await invoke("device_pair_acknowledge_request", { input });
    } catch (error) {
      console.warn("[device] reject request failed", error);
    }
    delete incomingExpectedCode.value[requestId];
    delete incomingAttemptCount.value[requestId];
    delete incomingCooldownUntil.value[requestId];
    incomingRequests.value = incomingRequests.value.filter((item) => item.request_id !== requestId);
  }

  async function submitPairCode(requestId: string, code: string): Promise<boolean> {
    const request = incomingRequests.value.find((item) => item.request_id === requestId);
    if (!request) return false;
    const normalized = code.replace(/\D/g, "").slice(0, 6);
    if (normalized.length !== 6) return false;
    const cooldownUntil = incomingCooldownUntil.value[requestId] ?? request.cooldown_until;
    if (cooldownUntil && Date.parse(cooldownUntil) > Date.now()) return false;
    const expectedCode = incomingExpectedCode.value[requestId];
    if (!expectedCode) return false;

    if (normalized !== expectedCode) {
      const nextAttempts = (incomingAttemptCount.value[requestId] ?? request.attempts ?? 0) + 1;
      incomingAttemptCount.value[requestId] = nextAttempts;
      if (nextAttempts >= 3) {
        incomingAttemptCount.value[requestId] = 0;
        incomingCooldownUntil.value[requestId] = new Date(Date.now() + 60_000).toISOString();
      }
      incomingRequests.value = incomingRequests.value.map((item) =>
        item.request_id !== requestId
          ? item
          : {
              ...item,
              attempts: incomingAttemptCount.value[requestId] ?? item.attempts,
              cooldown_until: incomingCooldownUntil.value[requestId] ?? item.cooldown_until,
            },
      );
      return false;
    }

    // PairComplete goes before anything is written here: if it fails, the
    // request stays, and the same code can be submitted again.
    try {
      await invoke("device_pair_complete_request", { input: { request_id: requestId } satisfies DevicePairRequestAckInput });
    } catch (error) {
      console.warn("[device] sending the completion failed; the request stays for a retry", error);
      return false;
    }

    // The peer has its side now. A write that fails here is tried again a
    // few times, and never reported as success.
    const RETRY_DELAYS_MS = [500, 1500, 3000];
    let saved = false;
    for (let attempt = 0; !saved; attempt++) {
      try {
        await savePairedDevice(
          request.from_device_id,
          request.from_hostname,
          requestId,
          request.from_bluetooth_address,
          request.via_bluetooth,
        );
        saved = true;
      } catch (error) {
        if (attempt >= RETRY_DELAYS_MS.length) {
          console.error("[device] saving the pair failed after the completion went to the peer", error);
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, RETRY_DELAYS_MS[attempt]));
      }
    }
    if (!saved) return false;

    delete incomingExpectedCode.value[requestId];
    delete incomingAttemptCount.value[requestId];
    delete incomingCooldownUntil.value[requestId];
    incomingRequests.value = incomingRequests.value.filter((item) => item.request_id !== requestId);
    pairCompletedAt.value = nowIso();
    return true;
  }

  async function unpairDevice(deviceId: string) {
    try {
      await invoke("device_unpair", { peerDeviceId: deviceId });
    } catch (error) {
      console.warn("[device] unpair failed", error);
    }
    delete channelStatusesByPeer.value[deviceId];
    await loadPairedDevices();
  }

  // Online: a channel holds a live connection to the device (green).
  function isDeviceOnline(device: PairedDevice): boolean {
    return (channelStatusesByPeer.value[device.peer_device_id] ?? []).some(
      (channel) => channel.color === ChannelColor.Green,
    );
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
    availableDevices,
    discoveredDevices,
    discoveredByChannel,
    discoveredWithPairedByChannel,
    incomingRequests,
    outgoingRequest,
    outgoingRequestSecondsLeft,
    addModeEnabled,
    debugStatus,
    pairCompletedAt,
    channelStatusesByPeer,
    hydrate,
    refreshDebugStatus,
    getChannelStatuses,
    refreshChannelStatuses,
    refreshAllChannelStatuses,
    setChannelEnabled,
    setPrimaryChannel,
    unlinkChannel,
    beginChannelSetup,
    getChannelSetup,
    refreshChannelSetup,
    endChannelSetup,
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
